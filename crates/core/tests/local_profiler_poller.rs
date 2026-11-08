//! Tier 1 (local server): the profile poller of a real runtime against a real MongoDB. A second
//! client named "myapp" runs the queries; the test turns the profiler on through the explicit
//! `Command::SetProfilingLevel` (the only path that does) and reads what the poller recorded back
//! through `Command::QueryShapes`. Writes only to throwaway `cbit_*` databases. Run with
//! `scripts/integration-tests.sh`.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bson::{Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ChartRange, ShapeSource, ShapeSummary};
use curlybrace_core::runtime::{
    Command, Event, RuntimeConfig, RuntimeHandle, SamplerConfig, SessionId,
    spawn_runtime_with_config,
};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::shape::from_query;
use curlybrace_core::store::{Namespace, ProfilingState, Query};
use mongodb::{Client, Collection};
use support::profiler::{APP, ProfileReset, app_client, docs, serial};
use support::{LocalDbs, local_enabled, local_mongo, next_event, no_repaint};

const LIMIT: Duration = Duration::from_secs(15);
/// Long enough for several one-second polls.
const SETTLE: Duration = Duration::from_millis(3500);
const DEADLINE: Duration = Duration::from_secs(30);

/// A throwaway database, a runtime polling every second, and the workload client.
struct Env {
    /// Declared first so the profiler is switched off before the database is dropped.
    _reset: ProfileReset,
    dbs: LocalDbs,
    rt: RuntimeHandle,
    _history_dir: tempfile::TempDir,
    app: Client,
    session: SessionId,
}

impl Env {
    async fn new() -> Self {
        let (dbs, _client, _store) = local_mongo().await;
        let _reset = ProfileReset { db: dbs.db.clone() };
        let history_dir = tempfile::tempdir().unwrap();
        let config = RuntimeConfig {
            command_timeout: LIMIT,
            history_path: Some(history_dir.path().join("history.sqlite")),
            sampler: SamplerConfig::default().with_interval(Duration::from_secs(1)),
            ..RuntimeConfig::default()
        };
        let rt = spawn_runtime_with_config(
            Some(Arc::new(MemorySecretStore::new())),
            None,
            no_repaint(),
            config,
        );
        Self {
            _reset,
            rt,
            _history_dir: history_dir,
            app: app_client(APP).await,
            session: SessionId::nil(),
            dbs,
        }
    }

    /// Connects the runtime (with the "curlybrace" marker) and remembers the session.
    async fn connect(&mut self) {
        let cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
        let event = self.answer(Command::Connect(cfg.clone()), "connect").await;
        assert!(matches!(event, Event::Connected { .. }), "{event:?}");
        self.session = cfg.id;
    }

    /// Sends `command` and returns its answer, skipping spontaneous background events.
    async fn answer(&mut self, command: Command, what: &str) -> Event {
        self.rt.tx.send(command).unwrap();
        loop {
            match next_event(&mut self.rt.rx, LIMIT, what).await {
                Event::Sampler { .. } | Event::Profiler { .. } => {}
                other => return other,
            }
        }
    }

    fn ns(&self) -> Namespace {
        self.dbs.ns("orders")
    }

    fn orders(&self) -> Collection<Document> {
        self.app.database(&self.dbs.db).collection("orders")
    }

    async fn seed(&self) {
        self.orders().insert_many(docs(300)).await.unwrap();
    }

    /// The app runs `finds` unindexed finds on `c` (one shape, many values).
    async fn app_finds(&self, finds: i32) {
        for i in 0..finds {
            let cursor = self.orders().find(doc! { "c": i }).await.unwrap();
            let _: Vec<Document> = futures::TryStreamExt::try_collect(cursor).await.unwrap();
        }
    }

    async fn set_level(&mut self, level: i32) -> ProfilingState {
        let command = Command::SetProfilingLevel {
            session: self.session,
            db: self.dbs.db.clone(),
            level,
        };
        match self.answer(command, "set profiling level").await {
            Event::ProfilingState { outcome, .. } => outcome.expect("profiling level set"),
            other => panic!("expected ProfilingState, got {other:?}"),
        }
    }

    async fn level(&mut self) -> ProfilingState {
        let command = Command::ProfilingState {
            session: self.session,
            db: self.dbs.db.clone(),
        };
        match self.answer(command, "profiling state").await {
            Event::ProfilingState { outcome, .. } => outcome.expect("profiling state"),
            other => panic!("expected ProfilingState, got {other:?}"),
        }
    }

    async fn shapes(&mut self) -> Vec<ShapeSummary> {
        let command = Command::QueryShapes {
            session: self.session,
            ns: self.ns(),
            range: ChartRange::Day,
            request: 0,
        };
        match self.answer(command, "query shapes").await {
            Event::QueryShapes { shapes, .. } => shapes,
            other => panic!("expected QueryShapes, got {other:?}"),
        }
    }

    /// Polls until `done` accepts the recorded shapes; panics with the last ones on timeout.
    async fn wait_for(
        &mut self,
        what: &str,
        done: impl Fn(&[ShapeSummary]) -> bool,
    ) -> Vec<ShapeSummary> {
        let deadline = Instant::now() + DEADLINE;
        loop {
            let shapes = self.shapes().await;
            if done(&shapes) {
                return shapes;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: last shapes {}",
                summarize(&shapes)
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

fn summarize(shapes: &[ShapeSummary]) -> String {
    let rows: Vec<_> = shapes
        .iter()
        .map(|s| (s.source, s.executions, s.misses, s.worst))
        .collect();
    format!("{rows:?}")
}

fn app_row(shapes: &[ShapeSummary]) -> Option<&ShapeSummary> {
    shapes.iter().find(|s| s.source == ShapeSource::App)
}

fn app_executions(shapes: &[ShapeSummary]) -> u64 {
    app_row(shapes).map_or(0, |s| s.executions)
}

fn c_shape_hash() -> String {
    from_query(&Query {
        filter: doc! { "c": 1 },
        ..Query::default()
    })
    .hash()
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn poller_records_app_queries_into_history() {
    if !local_enabled("poller_records_app_queries_into_history") {
        return;
    }
    let _serial = serial().await;
    let mut env = Env::new().await;
    env.seed().await;
    env.connect().await;
    let state = env.set_level(2).await;
    assert_eq!((state.level, state.slow_ms), (2, 100));

    env.app_finds(5).await;
    let shapes = env
        .wait_for("the app's five finds", |s| app_executions(s) >= 5)
        .await;
    let row = app_row(&shapes).unwrap();
    assert_eq!(
        (row.executions, row.misses),
        (5, 5),
        "{}",
        summarize(&shapes)
    );
    assert_eq!(row.worst, VerdictKind::CollectionScan);
    assert_eq!(
        row.hash,
        c_shape_hash(),
        "the shape a profile entry gives equals the typed one"
    );
    assert!(
        !row.shape_json.contains("\"c\":3"),
        "no literal value: {}",
        row.shape_json
    );

    // Our own traffic, through the marked runtime connection: a find and an explain of the same
    // shape. Neither may be counted as the app's.
    let find = Command::Find {
        session: env.session,
        ns: env.ns(),
        query: Query {
            filter: doc! { "c": 99 },
            ..Query::default()
        },
    };
    env.answer(find, "own find").await;
    let explain = Command::ExplainFind {
        session: env.session,
        ns: env.ns(),
        query: Query {
            filter: doc! { "c": 98 },
            ..Query::default()
        },
    };
    let event = env.answer(explain, "own explain").await;
    assert!(
        matches!(event, Event::Explain { outcome: Ok(_), .. }),
        "{event:?}"
    );
    tokio::time::sleep(SETTLE).await;
    let shapes = env.shapes().await;
    assert_eq!(
        app_executions(&shapes),
        5,
        "own traffic leaked: {}",
        summarize(&shapes)
    );
    let typed = shapes
        .iter()
        .find(|s| s.source == ShapeSource::Typed)
        .expect("the typed explain");
    assert_eq!(typed.hash, c_shape_hash());
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn poller_never_enables_the_profiler() {
    if !local_enabled("poller_never_enables_the_profiler") {
        return;
    }
    let _serial = serial().await;
    let mut env = Env::new().await;
    env.seed().await;
    env.connect().await;
    env.app_finds(5).await;
    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        env.level().await.level,
        0,
        "the poller must not turn the profiler on"
    );
    env.app_finds(5).await;
    tokio::time::sleep(SETTLE).await;
    assert_eq!(env.level().await.level, 0);
    assert!(
        env.shapes().await.is_empty(),
        "nothing is recorded while the profiler is off"
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn typed_explain_is_recorded_as_a_typed_shape() {
    if !local_enabled("typed_explain_is_recorded_as_a_typed_shape") {
        return;
    }
    let _serial = serial().await;
    let mut env = Env::new().await;
    env.seed().await;
    env.connect().await;
    let query = Query {
        filter: doc! { "c": 5 },
        ..Query::default()
    };
    let command = Command::ExplainFind {
        session: env.session,
        ns: env.ns(),
        query,
    };
    let verdict = match env.answer(command, "explain").await {
        Event::Explain { outcome, .. } => outcome.expect("explain"),
        other => panic!("expected Explain, got {other:?}"),
    };
    assert_eq!(verdict.kind, VerdictKind::CollectionScan);
    let shapes = env.wait_for("the typed shape", |s| !s.is_empty()).await;
    assert_eq!(shapes.len(), 1, "{}", summarize(&shapes));
    let row = &shapes[0];
    assert_eq!(row.source, ShapeSource::Typed);
    assert_eq!((row.executions, row.misses), (1, 1));
    assert_eq!(row.worst, VerdictKind::CollectionScan);
    assert_eq!(row.hash, c_shape_hash());
    assert_eq!(
        env.level().await.level,
        0,
        "an explain does not turn the profiler on"
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn poll_survives_the_profiler_being_turned_off_and_on() {
    if !local_enabled("poll_survives_the_profiler_being_turned_off_and_on") {
        return;
    }
    let _serial = serial().await;
    let mut env = Env::new().await;
    env.seed().await;
    env.connect().await;
    env.set_level(2).await;
    env.app_finds(3).await;
    env.wait_for("the first three", |s| app_executions(s) >= 3)
        .await;

    env.set_level(0).await;
    env.app_finds(2).await; // not profiled
    tokio::time::sleep(SETTLE).await;
    env.set_level(2).await;
    env.app_finds(4).await;
    let shapes = env
        .wait_for("seven in all", |s| app_executions(s) >= 7)
        .await;
    tokio::time::sleep(SETTLE).await;
    let shapes_later = env.shapes().await;
    assert_eq!(app_executions(&shapes), 7, "{}", summarize(&shapes));
    assert_eq!(
        app_executions(&shapes_later),
        7,
        "a duplicate appeared: {}",
        summarize(&shapes_later)
    );
    let gap: u64 = shapes_later
        .iter()
        .flat_map(|s| &s.buckets)
        .map(|b| b.gap_secs)
        .sum();
    assert_eq!(gap, 0, "turning the profiler off and on is not a loss");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn poller_records_a_gap_when_the_profile_rolls_off() {
    if !local_enabled("poller_records_a_gap_when_the_profile_rolls_off") {
        return;
    }
    let _serial = serial().await;
    let mut env = Env::new().await;
    env.seed().await;
    // A 16 KiB profile holds about ten entries.
    let db = env.app.database(&env.dbs.db);
    db.create_collection("system.profile")
        .capped(true)
        .size(16 * 1024)
        .await
        .unwrap();
    env.connect().await;
    env.set_level(2).await;
    env.app_finds(2).await;
    env.wait_for("the first two", |s| app_executions(s) >= 2)
        .await;

    // Bursts far bigger than the profile between two polls lose entries.
    let deadline = Instant::now() + DEADLINE;
    loop {
        env.app_finds(150).await;
        tokio::time::sleep(Duration::from_millis(1300)).await;
        let shapes = env.shapes().await;
        let gap: u64 = shapes
            .iter()
            .flat_map(|s| &s.buckets)
            .map(|b| b.gap_secs)
            .sum();
        if gap > 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no gap recorded: {}",
            summarize(&shapes)
        );
    }
}
