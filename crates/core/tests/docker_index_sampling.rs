//! The index usage sampler against a real server: a restart resets `$indexStats` and must show
//! up as a restart, never as a drop; an index recreated with another definition is a new series.
//! Tier 2 (docker): own `cb-it-*` container per test; run with `scripts/container-tests.sh`.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bson::doc;
use curlybrace_core::history::ChartRange;
use curlybrace_core::indexes::{IndexSpec, KeyKind};
use curlybrace_core::runtime::{
    Command, Event, IndexSeries, RuntimeConfig, RuntimeHandle, SamplerConfig, SessionId,
    spawn_runtime_with_config,
};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use support::{Container, docker_enabled, docker_mongo, next_event, no_repaint};

const LIMIT: Duration = Duration::from_secs(15);
/// Long enough for a paused sampler's backoff after a restart to run out.
const SAMPLE_WAIT: Duration = Duration::from_secs(90);

fn ns() -> Namespace {
    Namespace::new("cbit_sampling", "items")
}

fn spec(name: &str, field: &str) -> IndexSpec {
    mock_index_spec_named(name, &[(field, KeyKind::Asc)])
}

/// A runtime with a real history file in `dir` that samples every second.
fn runtime(dir: &tempfile::TempDir) -> RuntimeHandle {
    let config = RuntimeConfig {
        command_timeout: LIMIT,
        history_path: Some(dir.path().join("history.sqlite")),
        sampler: SamplerConfig::default().with_interval(Duration::from_secs(1)),
        ..RuntimeConfig::default()
    };
    spawn_runtime_with_config(
        Some(Arc::new(MemorySecretStore::new())),
        None,
        no_repaint(),
        config,
    )
}

/// Sends `command` and returns its answer, skipping the sampler's spontaneous status events.
async fn answer(rt: &mut RuntimeHandle, command: Command, what: &str) -> Event {
    rt.tx.send(command).unwrap();
    loop {
        match next_event(&mut rt.rx, LIMIT, what).await {
            Event::Sampler { .. } => {}
            other => return other,
        }
    }
}

async fn connect(rt: &mut RuntimeHandle, container: &Container) -> SessionId {
    let cfg = support::config(container);
    let event = answer(rt, Command::Connect(cfg.clone()), "connect").await;
    assert!(matches!(event, Event::Connected { .. }), "{event:?}");
    cfg.id
}

async fn create_index(rt: &mut RuntimeHandle, session: SessionId, spec: IndexSpec) {
    let command = Command::CreateIndex {
        session,
        ns: ns(),
        spec,
    };
    let event = answer(rt, command, "create index").await;
    assert!(matches!(event, Event::IndexChanged { .. }), "{event:?}");
}

/// Runs a query on `field` that the index on it serves. Failures are ignored on purpose: after a
/// restart the first queries may fail until the driver has reconnected.
async fn query_on(rt: &mut RuntimeHandle, session: SessionId, field: &str) {
    for value in 0..5_i32 {
        let command = Command::Find {
            session,
            ns: ns(),
            query: Query {
                filter: doc! { field: value },
                ..Query::default()
            },
        };
        answer(rt, command, "query").await;
    }
}

async fn history(rt: &mut RuntimeHandle, session: SessionId) -> Vec<IndexSeries> {
    let command = Command::IndexHistory {
        session,
        ns: ns(),
        range: ChartRange::Day,
        request: 0,
    };
    match answer(rt, command, "index history").await {
        // The `_id_` index exists on every collection and is not what these tests are about.
        Event::IndexHistory { series, .. } => series
            .into_iter()
            .filter(|s| s.index_name != "_id_")
            .collect(),
        other => panic!("expected IndexHistory, got {other:?}"),
    }
}

/// Polls `history` until `done` accepts it; panics with the last answer on timeout.
async fn wait_for_history(
    rt: &mut RuntimeHandle,
    session: SessionId,
    field: &str,
    what: &str,
    done: impl Fn(&[IndexSeries]) -> bool,
) -> Vec<IndexSeries> {
    let deadline = Instant::now() + SAMPLE_WAIT;
    loop {
        query_on(rt, session, field).await;
        let series = history(rt, session).await;
        if done(&series) {
            return series;
        }
        assert!(Instant::now() < deadline, "{what}: last history {series:?}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn total_uses(series: &[IndexSeries]) -> i64 {
    series.iter().flat_map(|s| &s.buckets).map(|b| b.uses).sum()
}

fn restarts(series: &[IndexSeries]) -> u32 {
    series
        .iter()
        .flat_map(|s| &s.buckets)
        .map(|b| b.restarts)
        .sum()
}

async fn seed(store: &impl DocumentStore) {
    for i in 0..10_i32 {
        store
            .insert(&ns(), doc! { "_id": i, "a": i, "b": i })
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn a_server_restart_shows_as_a_restart_point_not_a_drop() {
    if !docker_enabled("a_server_restart_shows_as_a_restart_point_not_a_drop") {
        return;
    }
    let (container, _client, store) = docker_mongo().await;
    seed(&store).await;
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(&dir);
    let session = connect(&mut rt, &container).await;
    create_index(&mut rt, session, spec("a_1", "a")).await;

    // Two samples of a counter that is above zero, so the deltas have something to lose.
    let before = wait_for_history(&mut rt, session, "a", "samples before the restart", |s| {
        s.len() == 1 && total_uses(s) >= 5
    })
    .await;
    let used_before = total_uses(&before);
    assert_eq!(restarts(&before), 0);

    container.stop();
    container.start();
    container.wait_tcp();

    // The sampler is paused while the server is away and resumes on its own.
    let after = wait_for_history(&mut rt, session, "a", "a restart point", |s| {
        restarts(s) >= 1
    })
    .await;
    assert_eq!(after.len(), 1, "a restart does not start a new series");
    let buckets: Vec<_> = after.iter().flat_map(|s| &s.buckets).collect();
    assert!(buckets.iter().all(|b| b.uses >= 0), "negative bucket");
    assert!(
        total_uses(&after) >= used_before,
        "the total dropped across the restart: {} < {used_before}",
        total_uses(&after)
    );
    let latest = after[0].latest.expect("a newest sample");
    assert!(latest.at_secs > 0);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn a_dropped_and_recreated_index_starts_a_new_series() {
    if !docker_enabled("a_dropped_and_recreated_index_starts_a_new_series") {
        return;
    }
    let (container, _client, store) = docker_mongo().await;
    seed(&store).await;
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(&dir);
    let session = connect(&mut rt, &container).await;

    let first = spec("lookup", "a");
    create_index(&mut rt, session, first.clone()).await;
    wait_for_history(&mut rt, session, "a", "a sample of the first index", |s| {
        s.len() == 1
    })
    .await;

    let drop = Command::DropIndex {
        session,
        ns: ns(),
        name: "lookup".into(),
    };
    let dropped = answer(&mut rt, drop, "drop index").await;
    assert!(matches!(dropped, Event::IndexChanged { .. }), "{dropped:?}");
    // Same name, other keys: not the same index.
    let second = spec("lookup", "b");
    create_index(&mut rt, session, second.clone()).await;

    // The tab draws the indexes that are listed: the dropped definition's series is kept in the
    // history but not returned, so the one series is the new definition's.
    let wanted = second.definition_hash();
    let series = wait_for_history(&mut rt, session, "b", "the second definition", |s| {
        s.len() == 1 && s[0].definition_hash == wanted
    })
    .await;
    assert_ne!(first.definition_hash(), wanted);
    assert_eq!(series[0].index_name, "lookup");
    assert!(
        series
            .iter()
            .all(|s| restarts(std::slice::from_ref(s)) == 0)
    );
}
