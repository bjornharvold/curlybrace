//! The profile poller: the pure fold of profile entries into shape observations, and the task
//! that reads `system.profile` of every database whose profiler is on.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bson::{Bson, Document, doc};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::sync::watch;
use tokio::time::Instant;
use uuid::Uuid;

use super::profiler::{fold_entries, run};
use super::profiler_status::ProfilerStatus;
use super::sampler::{SamplerConfig, SamplerPause};
use super::*;
use crate::error::CoreError;
use crate::explain::VerdictKind;
use crate::history::{ChartRange, HistoryDb, ShapeObservation, ShapeSource, ShapeSummary};
use crate::profile::{OWN_APP_NAME, ProfileEntry};
use crate::store::{Namespace, Page, ProfileRead, ProfilingState, Query};
use crate::test_support::InMemoryStore;

const CONN: Uuid = Uuid::from_u128(9);
const HOST: &str = "localhost:27017";
const NOW: i64 = 1_800_000_000;
const BUCKET: i64 = 600;
/// An hour before `NOW`, which is also the start of a 10 minute bucket.
const HOUR_AGO_MS: i64 = (NOW - 3_600) * 1_000;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---------------------------------------------------------------- entries

fn entry_with(ts_millis: i64, ns: &str, command: Document) -> ProfileEntry {
    ProfileEntry {
        ts_millis,
        ts_secs: ts_millis.div_euclid(1_000),
        ns: ns.into(),
        op: "query".into(),
        millis: 7,
        plan_summary: "COLLSCAN".into(),
        keys_examined: 0,
        docs_examined: 1_000,
        returned: 1,
        has_sort_stage: false,
        app_name: None,
        command,
    }
}

fn entry_on(ts_millis: i64, ns: &str, filter: Document) -> ProfileEntry {
    let collection = ns.split_once('.').map_or(ns, |(_, c)| c).to_owned();
    entry_with(ts_millis, ns, doc! { "find": collection, "filter": filter })
}

/// A collection scan of `shop.orders` filtered on `status`.
fn scan(ts_millis: i64) -> ProfileEntry {
    entry_on(ts_millis, "shop.orders", doc! { "status": "open" })
}

fn fold(entries: &[ProfileEntry]) -> Vec<ShapeObservation> {
    fold_entries(CONN, HOST, "shop", entries, BUCKET)
}

// ------------------------------------------------------------------- fold

#[test]
fn groups_by_shape_and_bucket() {
    let by_status = |ts, v: &str| entry_on(ts, "shop.orders", doc! { "status": v });
    let entries = vec![
        by_status(1_200_000, "a"),
        by_status(1_500_000, "b"),
        by_status(1_800_000, "c"),
        entry_on(1_200_000, "shop.orders", doc! { "sku": "x" }),
    ];
    let out = fold(&entries);
    assert_eq!(out.len(), 3, "two shapes, one of them in two buckets");
    let counts: Vec<(i64, u64)> = out.iter().map(|o| (o.at_secs, o.executions)).collect();
    assert!(counts.contains(&(1_200, 2)), "{counts:?}");
    assert!(counts.contains(&(1_200, 1)), "{counts:?}");
    assert!(counts.contains(&(1_800, 1)), "{counts:?}");
    for o in &out {
        assert_eq!(o.key.connection, CONN);
        assert_eq!(o.key.host, HOST);
        assert_eq!(o.key.db, "shop");
        assert_eq!(o.key.collection, "orders");
        assert_eq!(o.key.source, ShapeSource::App);
        assert_eq!(o.at_secs % BUCKET, 0, "the bucket start");
        assert_eq!(o.gap_secs, 0);
    }
}

#[test]
fn own_traffic_is_dropped() {
    let ours = ProfileEntry {
        app_name: Some(OWN_APP_NAME.into()),
        ..scan(1_000)
    };
    let prefixed = ProfileEntry {
        app_name: Some(format!("myapp {OWN_APP_NAME}")),
        ..scan(2_000)
    };
    let own_ns = entry_on(3_000, "shop.system.profile", doc! { "ts": 1 });
    assert!(fold(&[ours, prefixed, own_ns]).is_empty());
    let theirs = ProfileEntry {
        app_name: Some("billing".into()),
        ..scan(4_000)
    };
    assert_eq!(fold(&[theirs]).len(), 1);
}

#[test]
fn continuations_are_dropped() {
    let more = ProfileEntry {
        op: "getmore".into(),
        ..entry_with(
            1_000,
            "shop.orders",
            doc! { "getMore": 7_i64, "collection": "orders" },
        )
    };
    assert!(fold(&[more]).is_empty());
}

#[test]
fn counts_misses_and_worst_verdict() {
    let indexed = ProfileEntry {
        plan_summary: "IXSCAN { status: 1 }".into(),
        keys_examined: 1,
        docs_examined: 1,
        millis: 3,
        ..scan(2_000)
    };
    let slow = ProfileEntry {
        millis: 90,
        ..scan(3_000)
    };
    let out = fold(&[scan(1_000), indexed, slow]);
    assert_eq!(out.len(), 1);
    let o = &out[0];
    assert_eq!(o.executions, 3);
    assert_eq!(o.misses, 2, "the two collection scans");
    assert_eq!(o.total_millis, 7 + 3 + 90);
    assert_eq!(o.max_millis, 90);
    assert_eq!(o.worst, VerdictKind::CollectionScan);
}

#[test]
fn no_literal_value_reaches_an_observation() {
    let secret = entry_with(
        1_000,
        "shop.orders",
        doc! {
            "find": "orders",
            "filter": { "email": "zebra-secret@example.com", "n": 424_242_i64,
                        "tags": { "$in": ["quokka-1", "quokka-2"] } },
            "sort": { "created": -1 },
        },
    );
    let out = fold(&[secret]);
    assert_eq!(out.len(), 1);
    let dump = format!("{out:?}");
    for literal in ["zebra-secret", "424242", "quokka"] {
        assert!(!dump.contains(literal), "{literal} leaked: {dump}");
    }
    assert!(out[0].shape_json.contains("email"), "field names stay");
}

#[test]
fn entries_for_other_databases_are_ignored() {
    let other = entry_on(1_000, "other.orders", doc! { "status": "open" });
    assert!(fold(&[other]).is_empty());
}

#[test]
fn aggregate_without_leading_match_is_skipped() {
    let mut piped = entry_with(
        1_000,
        "shop.orders",
        doc! { "aggregate": "orders",
        "pipeline": [{ "$group": { "_id": "$a" } }, { "$match": { "x": 1 } }] },
    );
    piped.op = "command".into();
    assert!(fold(&[piped]).is_empty(), "no shape, no observation");
}

#[test]
fn output_is_deterministic() {
    let a = entry_on(1_200_000, "shop.orders", doc! { "status": "x" });
    let b = entry_on(1_900_000, "shop.carts", doc! { "owner": "x" });
    let c = entry_on(1_300_000, "shop.orders", doc! { "sku": "x" });
    let d = entry_on(1_200_000, "shop.carts", doc! { "owner": "x" });
    let forward = fold(&[a.clone(), b.clone(), c.clone(), d.clone()]);
    let backward = fold(&[d, c, b, a]);
    assert_eq!(format!("{forward:?}"), format!("{backward:?}"));
    let order: Vec<(String, i64)> = forward
        .iter()
        .map(|o| (o.key.collection.clone(), o.at_secs))
        .collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted, "sorted by collection, then bucket");
}

// ----------------------------------------------------------------- poller

/// A boundary double around `InMemoryStore`: counts the profiler calls, can swap the store under
/// the poller and fail the level call.
struct Probe {
    inner: Mutex<Arc<InMemoryStore>>,
    dbs: Vec<String>,
    levels: Mutex<Vec<String>>,
    reads: Mutex<Vec<(String, i64)>>,
    sets: AtomicUsize,
    level_failure: Mutex<Option<CoreError>>,
}

impl Probe {
    fn new(dbs: &[&str], inner: InMemoryStore) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Arc::new(inner)),
            dbs: dbs.iter().map(|d| (*d).to_owned()).collect(),
            levels: Mutex::default(),
            reads: Mutex::default(),
            sets: AtomicUsize::new(0),
            level_failure: Mutex::default(),
        })
    }

    fn store(&self) -> Arc<InMemoryStore> {
        self.inner.lock().unwrap().clone()
    }

    fn swap(&self, inner: InMemoryStore) {
        *self.inner.lock().unwrap() = Arc::new(inner);
    }

    fn fail_levels(&self, error: Option<CoreError>) {
        *self.level_failure.lock().unwrap() = error;
    }

    fn read_calls(&self) -> Vec<(String, i64)> {
        self.reads.lock().unwrap().clone()
    }

    fn read_count(&self) -> usize {
        self.reads.lock().unwrap().len()
    }

    fn level_calls(&self) -> Vec<String> {
        self.levels.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl DocumentStore for Probe {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(self.dbs.clone())
    }
    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.store().list_collections(db).await
    }
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.store().find(ns, q).await
    }
    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        self.store().insert(ns, doc).await
    }
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        self.store().replace(ns, id, doc).await
    }
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.store().delete(ns, id).await
    }
    async fn profiling_level(&self, db: &str) -> Result<ProfilingState> {
        self.levels.lock().unwrap().push(db.to_owned());
        if let Some(error) = self.level_failure.lock().unwrap().clone() {
            return Err(error);
        }
        self.store().profiling_level(db).await
    }
    async fn read_profile(&self, db: &str, since_ts_millis: i64) -> Result<ProfileRead> {
        self.reads
            .lock()
            .unwrap()
            .push((db.to_owned(), since_ts_millis));
        self.store().read_profile(db, since_ts_millis).await
    }
    async fn set_profiling_level(&self, _db: &str, _level: i32) -> Result<ProfilingState> {
        self.sets.fetch_add(1, Ordering::SeqCst);
        Err(CoreError::Unsupported("the poller must not write".into()))
    }
}

fn history() -> Arc<HistoryDb> {
    Arc::new(HistoryDb::open_in_memory().unwrap())
}

/// A wall clock that follows tokio's (paused) time, starting at `NOW`.
fn clock() -> impl Fn() -> i64 + Send + Sync + 'static {
    let began = Instant::now();
    move || NOW + began.elapsed().as_secs() as i64
}

struct Running {
    handle: tokio::task::JoinHandle<()>,
    status: UnboundedReceiver<ProfilerStatus>,
    cfg: watch::Sender<SamplerConfig>,
}

fn start(probe: &Arc<Probe>, history: &Arc<HistoryDb>, cfg: SamplerConfig) -> Running {
    let (cfg_tx, cfg_rx) = watch::channel(cfg);
    let (status_tx, status) = unbounded_channel();
    let store: Arc<dyn DocumentStore> = probe.clone();
    let handle = tokio::spawn(run(
        store,
        history.clone(),
        CONN,
        cfg_rx,
        status_tx,
        clock(),
    ));
    Running {
        handle,
        status,
        cfg: cfg_tx,
    }
}

/// Lets the poller work until `done` holds. Virtual time moves a second at a time, so a tick
/// (60 s) is reached after at most 60 rounds; the real sleep gives the blocking pool its turn.
async fn until(what: &str, done: impl Fn() -> bool) {
    for _ in 0..3_000 {
        if done() {
            return;
        }
        tokio::time::sleep(secs(1)).await;
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("never happened: {what}");
}

async fn next_status(running: &mut Running) -> ProfilerStatus {
    tokio::time::timeout(secs(1_000), running.status.recv())
        .await
        .expect("a status in time")
        .expect("channel open")
}

fn totals(history: &HistoryDb, db: &str, collection: &str) -> (u64, u64) {
    let shapes = summaries(history, db, collection);
    (
        shapes.iter().map(|s| s.executions).sum(),
        shapes.iter().map(|s| s.misses).sum(),
    )
}

fn summaries(history: &HistoryDb, db: &str, collection: &str) -> Vec<ShapeSummary> {
    history
        .shapes_for(CONN, db, collection, ChartRange::Day, NOW + 120)
        .unwrap()
}

fn store_of(db: &str, level: i32, entries: Vec<ProfileEntry>) -> InMemoryStore {
    InMemoryStore::new()
        .with_profiling_level(db, level)
        .with_profile(db, entries)
}

fn at(offset_ms: i64) -> i64 {
    HOUR_AGO_MS + offset_ms
}

#[tokio::test(start_paused = true)]
async fn reads_only_databases_with_the_profiler_on() {
    let inner = store_of("shop", 1, vec![scan(at(1)), scan(at(2))])
        .with_profiling_level("quiet", 0)
        .with_profile("quiet", vec![entry_on(at(1), "quiet.t", doc! { "a": 1 })]);
    let probe = Probe::new(&["shop", "quiet"], inner);
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());

    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    let reads: BTreeSet<String> = probe.read_calls().into_iter().map(|(d, _)| d).collect();
    assert_eq!(reads, BTreeSet::from(["shop".to_owned()]));
    assert_eq!(totals(&db, "shop", "orders"), (2, 2));
    assert_eq!(totals(&db, "quiet", "t"), (0, 0));
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn never_enables_the_profiler() {
    let inner = store_of("shop", 0, vec![scan(at(1))]).with_profiling_level("other", 1);
    let probe = Probe::new(&["shop", "other"], inner);
    let running = start(&probe, &history(), SamplerConfig::default());
    until("four ticks", || probe.level_calls().len() >= 8).await;
    assert_eq!(probe.sets.load(Ordering::SeqCst), 0);
    assert_eq!(
        probe.store().profiling_level("shop").await.unwrap().level,
        0,
        "still off"
    );
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn watermark_prevents_double_counting_across_ticks() {
    let probe = Probe::new(
        &["shop"],
        store_of("shop", 1, vec![scan(at(5)), scan(at(9))]),
    );
    let db = history();
    let running = start(&probe, &db, SamplerConfig::default());
    until("the fourth read", || probe.read_count() >= 4).await;

    assert_eq!(totals(&db, "shop", "orders"), (2, 2), "counted once");
    let marks: Vec<i64> = probe.read_calls().iter().map(|(_, m)| *m).collect();
    assert_eq!(
        marks[1..],
        [at(9); 3],
        "the watermark sits at the newest entry"
    );
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn full_page_is_read_again_immediately() {
    let entries: Vec<ProfileEntry> = (1..=1_500).map(|i| scan(at(i))).collect();
    let probe = Probe::new(&["shop"], store_of("shop", 1, entries));
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());

    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    assert_eq!(probe.read_count(), 2, "two reads, one tick");
    assert_eq!(totals(&db, "shop", "orders").0, 1_500);
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn unparseable_page_does_not_stall_the_watermark() {
    let broken: Vec<i64> = (1..=1_000).map(at).collect();
    let inner = store_of("shop", 1, (1001..=1005).map(|i| scan(at(i))).collect())
        .with_profile_unparseable("shop", broken);
    let probe = Probe::new(&["shop"], inner);
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());

    next_status(&mut running).await;
    assert_eq!(probe.read_count(), 2);
    assert_eq!(totals(&db, "shop", "orders").0, 5, "past the broken page");
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn roll_off_records_a_gap_and_reports_missed_entries() {
    let probe = Probe::new(
        &["shop"],
        store_of("shop", 1, vec![scan(at(5)), scan(at(9))]),
    );
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    assert!(
        summaries(&db, "shop", "orders")
            .iter()
            .all(|s| s.buckets.iter().all(|b| b.gap_secs == 0))
    );

    // The capped collection rolled everything up to half an hour later off before the next read.
    let later = at(1_800_000);
    probe.swap(store_of("shop", 1, vec![scan(later), scan(later + 1)]));

    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::MissedEntries { db: "shop".into() }
    );
    let gap: u64 = summaries(&db, "shop", "orders")
        .iter()
        .flat_map(|s| s.buckets.iter())
        .map(|b| b.gap_secs)
        .sum();
    assert!(gap > 0, "the loss is on the record");
    assert_eq!(totals(&db, "shop", "orders").0, 4, "what survived is read");
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn first_sight_ignores_entries_older_than_raw_retention() {
    let old = (NOW - 49 * 3_600) * 1_000;
    let probe = Probe::new(
        &["shop"],
        store_of("shop", 1, vec![scan(old), scan(old + 1), scan(at(1))]),
    );
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());
    next_status(&mut running).await;
    assert_eq!(totals(&db, "shop", "orders").0, 1, "only the recent one");
    assert!(
        probe.read_calls()[0].1 >= (NOW - 48 * 3_600) * 1_000 - 1_000,
        "{:?}",
        probe.read_calls()
    );
    assert_eq!(
        next_status_or_quiet(&mut running).await,
        None,
        "an old first entry is not a gap"
    );
    running.handle.abort();
}

/// The next status if one arrives within three ticks, else `None`.
async fn next_status_or_quiet(running: &mut Running) -> Option<ProfilerStatus> {
    tokio::time::timeout(secs(200), running.status.recv())
        .await
        .ok()
        .flatten()
}

#[tokio::test(start_paused = true)]
async fn read_only_store_still_reads_and_records() {
    let inner: Arc<dyn DocumentStore> = crate::read_only_store::ReadOnlyStore::wrap(
        Arc::new(store_of("shop", 1, vec![scan(at(1))]).with_docs(
            &Namespace::new("shop", "orders"),
            vec![crate::test_support::mock_objects::mock_doc()],
        )),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    let db = history();
    let (status_tx, mut status) = unbounded_channel();
    let (_cfg, cfg_rx) = watch::channel(SamplerConfig::default());
    let task = tokio::spawn(run(inner, db.clone(), CONN, cfg_rx, status_tx, clock()));
    let first = tokio::time::timeout(secs(500), status.recv()).await;
    assert_eq!(first, Ok(Some(ProfilerStatus::Reading { databases: 1 })));
    assert_eq!(totals(&db, "shop", "orders"), (1, 1));
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn unauthorised_db_is_skipped_and_all_unauthorised_pauses() {
    let inner =
        store_of("shop", 1, vec![scan(at(1))]).with_profile_error("audit", CoreError::Unauthorized);
    let probe = Probe::new(&["audit", "shop"], inner);
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    assert_eq!(totals(&db, "shop", "orders").0, 1);
    running.handle.abort();

    let denied = store_of("shop", 1, vec![]).with_profile_error("shop", CoreError::AuthFailed);
    let probe = Probe::new(&["shop"], denied);
    let mut running = start(&probe, &history(), SamplerConfig::default());
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Paused(SamplerPause::NotAuthorized)
    );
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn mongos_unsupported_is_quiet() {
    let mongos = CoreError::Unsupported(crate::store::PROFILER_MONGOS_MESSAGE.into());
    let inner = InMemoryStore::new().with_profile_error("shop", mongos);
    let probe = Probe::new(&["shop"], inner);
    let mut running = start(&probe, &history(), SamplerConfig::default());
    until("three ticks", || probe.level_calls().len() >= 3).await;
    assert_eq!(probe.read_count(), 0);
    assert!(
        running.status.try_recv().is_err(),
        "nothing readable is idle, which needs no report"
    );
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn connection_error_pauses_and_recovers() {
    let probe = Probe::new(&["shop"], store_of("shop", 1, vec![scan(at(1))]));
    probe.fail_levels(Some(CoreError::Unreachable));
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Paused(SamplerPause::ConnectionLost)
    );
    probe.fail_levels(None);
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    assert_eq!(totals(&db, "shop", "orders").0, 1);
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn excluded_and_system_databases_are_skipped() {
    let inner = ["admin", "local", "config", "skipme", "shop"]
        .iter()
        .fold(InMemoryStore::new(), |s, d| s.with_profiling_level(d, 1));
    let probe = Probe::new(&["admin", "local", "config", "skipme", "shop"], inner);
    let cfg = SamplerConfig {
        excluded_dbs: BTreeSet::from(["skipme".to_owned()]),
        ..SamplerConfig::default()
    };
    let mut running = start(&probe, &history(), cfg);
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    assert_eq!(probe.level_calls(), vec!["shop".to_owned()]);
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn stops_when_the_status_channel_closes() {
    let probe = Probe::new(&["shop"], store_of("shop", 1, vec![scan(at(1))]));
    let Running {
        handle,
        status,
        cfg: _cfg,
    } = start(&probe, &history(), SamplerConfig::default());
    until("a first read", || probe.read_count() >= 1).await;
    drop(status);
    tokio::time::timeout(secs(500), handle)
        .await
        .expect("the task ends")
        .expect("without a panic");
}

// ------------------------------------------------------ session attachment

#[tokio::test]
async fn stale_generation_attach_aborts() {
    let session = super::session::Session::new(1, None);
    let poller = tokio::spawn(std::future::pending::<()>());
    let abort = poller.abort_handle();
    session.move_generation_for_tests(2);
    assert!(!session.attach_profiler(1, poller), "a stale epoch");
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(abort.is_finished());

    let current = tokio::spawn(std::future::pending::<()>());
    let abort = current.abort_handle();
    assert!(session.attach_profiler(2, current));
    session.begin(3);
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(abort.is_finished(), "a newer epoch stops the poller");
}

// ------------------------------------------------------------ runtime wiring

struct FixedConnector(Arc<dyn DocumentStore>);

#[async_trait::async_trait]
impl Connector for FixedConnector {
    async fn connect(
        &self,
        _cfg: &crate::connection::ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn a_connected_session_forwards_the_poller_status_as_an_event() {
    let dir = tempfile::tempdir().unwrap();
    let ns = Namespace::new("shop", "orders");
    let recent = crate::clock::now_secs() * 1_000 - 1_000;
    let store = store_of("shop", 1, vec![scan(recent)])
        .with_docs(&ns, vec![crate::test_support::mock_objects::mock_doc()]);
    let config = RuntimeConfig {
        history_path: Some(dir.path().join("history.sqlite")),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(Arc::new(store))),
        Arc::new(|| {}),
        config,
    );
    let cfg = crate::connection::ConnectionConfig::new("c", "mongodb://localhost:27017");
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    let seen = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::Profiler { session, status } = h.rx.recv().await.expect("channel open") {
                return (session, status);
            }
        }
    })
    .await
    .expect("a profiler status in time");
    assert_eq!(seen, (cfg.id, ProfilerStatus::Reading { databases: 1 }));
}
