//! A pipeline explained in the Aggregate tab is recorded as a typed shape, with the verdict of
//! its input plan; previews are never explained, and nothing about the recording may change or
//! delay the answer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bson::{Document, doc};

use super::aggregate_record::{pipeline_verdict, record_pipeline};
use super::*;
use crate::clock;
use crate::connection::ConnectionConfig;
use crate::explain::{AggPlanOutcome, ExplainVerbosity, VerdictKind, parse_aggregate_explain};
use crate::history::{ChartRange, HistoryDb, ShapeSource, ShapeSummary};
use crate::log_capture::{RECORDS, capture, records_mentioning};
use crate::pipeline::{CollationOption, HintOption, PipelineOptions};
use crate::shape::{Shape, from_profile_command};
use crate::store::Namespace;
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

const WAIT: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_millis(400);
const SECRET: &str = "zebra-secret@example.com";

struct FixedConnector(Arc<dyn DocumentStore>);

#[async_trait::async_trait]
impl Connector for FixedConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0.clone())
    }
}

fn stages() -> Vec<Document> {
    vec![
        doc! { "$match": { "email": SECRET, "age": { "$gt": 424_242 } } },
        doc! { "$sort": { "age": -1 } },
        doc! { "$group": { "_id": "$city" } },
    ]
}

fn profiler_shape(stages: &[Document]) -> Shape {
    let pipeline: Vec<bson::Bson> = stages.iter().cloned().map(bson::Bson::Document).collect();
    from_profile_command(
        "command",
        &doc! { "aggregate": "orders", "pipeline": pipeline },
    )
    .expect("a leading $match is a shape")
}

fn planner(winning: Document) -> Document {
    doc! { "queryPlanner": { "winningPlan": winning }, "ok": 1 }
}

fn collscan_explain() -> Document {
    planner(doc! { "stage": "COLLSCAN", "direction": "forward" })
}

fn sort_explain() -> Document {
    planner(
        doc! { "stage": "SORT", "sortPattern": { "age": -1 }, "inputStage": {
        "stage": "FETCH", "inputStage": {
            "stage": "IXSCAN", "keyPattern": { "email": 1 }, "indexName": "email_1" } } },
    )
}

fn ixscan_explain(keys: Document) -> Document {
    planner(doc! { "stage": "FETCH", "inputStage": {
    "stage": "IXSCAN", "keyPattern": keys, "indexName": "by_keys" } })
}

fn verdict_of(explain: &Document, shape: &Shape) -> Option<VerdictKind> {
    pipeline_verdict(&parse_aggregate_explain(explain), shape)
}

#[test]
fn collscan_input_is_a_collection_scan() {
    let shape = profiler_shape(&stages());
    assert_eq!(
        verdict_of(&collscan_explain(), &shape),
        Some(VerdictKind::CollectionScan)
    );
}

#[test]
fn sort_in_input_plan_is_sort_not_covered() {
    let shape = profiler_shape(&stages());
    assert_eq!(
        verdict_of(&sort_explain(), &shape),
        Some(VerdictKind::SortNotCovered)
    );
}

#[test]
fn a_good_index_is_index_used_and_a_range_first_key_is_field_order() {
    let shape = profiler_shape(&[
        doc! { "$match": { "status": "open", "age": { "$gt": 3 } } },
        doc! { "$group": { "_id": 1 } },
    ]);
    assert_eq!(
        verdict_of(&ixscan_explain(doc! { "status": 1, "age": 1 }), &shape),
        Some(VerdictKind::IndexUsed)
    );
    assert_eq!(
        verdict_of(&ixscan_explain(doc! { "age": 1, "status": 1 }), &shape),
        Some(VerdictKind::FieldOrder)
    );
}

#[test]
fn an_unfiltered_scan_is_no_filter_and_never_inefficient() {
    let shape = profiler_shape(&[doc! { "$match": {} }, doc! { "$group": { "_id": 1 } }]);
    assert_eq!(
        verdict_of(&collscan_explain(), &shape),
        Some(VerdictKind::NoFilter)
    );
}

#[test]
fn nothing_to_judge_gives_no_verdict() {
    let shape = profiler_shape(&stages());
    assert_eq!(
        pipeline_verdict(&AggPlanOutcome::NotUnderstood, &shape),
        None
    );
    assert_eq!(verdict_of(&doc! { "ok": 1 }, &shape), None);
}

#[test]
fn a_sharded_plan_records_nothing() {
    let sharded = crate::ejson::editor_json_to_doc(include_str!(
        "../../tests/fixtures/explain_aggregate/sharded_stages.json"
    ))
    .expect("fixture parses");
    let shape = profiler_shape(&stages());
    assert_eq!(verdict_of(&sharded, &shape), None);
}

struct Setup {
    h: RuntimeHandle,
    session: SessionId,
    path: PathBuf,
    _dir: tempfile::TempDir,
}

async fn next(h: &mut RuntimeHandle) -> Event {
    loop {
        let event = tokio::time::timeout(WAIT, h.rx.recv())
            .await
            .expect("event within bound")
            .expect("channel open");
        if !matches!(event, Event::Sampler { .. } | Event::Profiler { .. }) {
            return event;
        }
    }
}

async fn connected(store: InMemoryStore) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let config = RuntimeConfig {
        history_path: Some(path.clone()),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(FixedConnector(Arc::new(store))),
        Arc::new(|| {}),
        config,
    );
    let cfg = ConnectionConfig::new("c", "mongodb://localhost:27017");
    h.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    Setup {
        h,
        session: cfg.id,
        path,
        _dir: dir,
    }
}

fn explain_command(session: SessionId, stages: Vec<Document>, options: PipelineOptions) -> Command {
    Command::AggregateExplain {
        session,
        request: 5,
        ns: mock_namespace(),
        stages,
        verbosity: ExplainVerbosity::QueryPlanner,
        options,
    }
}

async fn explained(setup: &mut Setup, stages: Vec<Document>, options: PipelineOptions) -> bool {
    setup
        .h
        .tx
        .send(explain_command(setup.session, stages, options))
        .unwrap();
    match next(&mut setup.h).await {
        Event::AggregateExplained { outcome, .. } => outcome.is_ok(),
        other => panic!("expected AggregateExplained, got {other:?}"),
    }
}

fn typed_rows(path: &Path, session: SessionId) -> Vec<ShapeSummary> {
    let ns = mock_namespace();
    HistoryDb::open(path)
        .unwrap()
        .0
        .shapes_for(
            session,
            &ns.db,
            &ns.collection,
            ChartRange::Day,
            clock::now_secs() + 60,
        )
        .unwrap()
}

async fn recorded(setup: &Setup) -> Vec<ShapeSummary> {
    for _ in 0..100 {
        let rows = typed_rows(&setup.path, setup.session);
        if !rows.is_empty() {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Vec::new()
}

async fn nothing_recorded(setup: &Setup) -> bool {
    tokio::time::sleep(SETTLE).await;
    typed_rows(&setup.path, setup.session).is_empty()
}

fn scripted() -> InMemoryStore {
    InMemoryStore::new().with_aggregate_explain(collscan_explain())
}

#[tokio::test]
async fn an_explained_match_pipeline_is_recorded_as_typed() {
    let mut setup = connected(scripted()).await;
    assert!(explained(&mut setup, stages(), PipelineOptions::default()).await);
    let rows = recorded(&setup).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].source, ShapeSource::Typed);
    assert_eq!((rows[0].executions, rows[0].misses), (1, 1));
    assert_eq!(rows[0].worst, VerdictKind::CollectionScan);
}

#[tokio::test]
async fn the_pipeline_shape_equals_the_profiler_shape_of_the_same_pipeline() {
    let mut setup = connected(scripted()).await;
    assert!(explained(&mut setup, stages(), PipelineOptions::default()).await);
    let rows = recorded(&setup).await;
    let shape = profiler_shape(&stages());
    assert_eq!(rows[0].hash, shape.hash());
    assert_eq!(rows[0].shape_json, shape.json());
}

#[tokio::test]
async fn the_recording_holds_no_literal() {
    let mut setup = connected(scripted()).await;
    assert!(explained(&mut setup, stages(), PipelineOptions::default()).await);
    let rows = recorded(&setup).await;
    let dump = format!("{rows:?}");
    assert!(
        !dump.contains("zebra-secret") && !dump.contains("424242"),
        "{dump}"
    );
}

#[tokio::test]
async fn a_pipeline_not_starting_with_match_is_not_recorded() {
    let mut setup = connected(scripted()).await;
    let lead_group = vec![doc! { "$group": { "_id": "$city" } }];
    assert!(explained(&mut setup, lead_group, PipelineOptions::default()).await);
    assert!(nothing_recorded(&setup).await);
}

#[tokio::test]
async fn a_hint_or_collation_records_nothing() {
    let mut setup = connected(scripted()).await;
    let hinted = PipelineOptions {
        hint: Some(HintOption::Name("by_email".into())),
        ..PipelineOptions::default()
    };
    assert!(explained(&mut setup, stages(), hinted).await);
    assert!(nothing_recorded(&setup).await);
}

#[tokio::test]
async fn a_collation_records_nothing() {
    let mut setup = connected(scripted()).await;
    let collated = PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: None,
            case_level: None,
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        ..PipelineOptions::default()
    };
    assert!(explained(&mut setup, stages(), collated).await);
    assert!(nothing_recorded(&setup).await);
}

#[tokio::test]
async fn other_options_still_record() {
    let mut setup = connected(scripted()).await;
    let options = PipelineOptions {
        allow_disk_use: Some(true),
        max_time_ms: Some(2_000),
        ..PipelineOptions::default()
    };
    assert!(explained(&mut setup, stages(), options).await);
    assert_eq!(recorded(&setup).await.len(), 1);
}

#[tokio::test]
async fn a_failed_explain_records_nothing() {
    let mut setup = connected(InMemoryStore::new()).await;
    assert!(!explained(&mut setup, stages(), PipelineOptions::default()).await);
    assert!(nothing_recorded(&setup).await);
}

#[tokio::test]
async fn recording_never_delays_the_event() {
    let mut setup = connected(scripted()).await;
    // Another connection holds the write lock for the whole test: the recording blocks on it.
    let holder = rusqlite::Connection::open(&setup.path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    assert!(explained(&mut setup, stages(), PipelineOptions::default()).await);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the event waited"
    );
    holder.execute_batch("ROLLBACK").unwrap();
}

#[tokio::test]
async fn nothing_about_the_pipeline_is_logged() {
    capture();
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    history.exec_for_tests("ALTER TABLE shape RENAME TO shape_gone");
    let ns: Namespace = mock_namespace();
    record_pipeline(
        &history,
        SessionId::from_u128(7),
        &ns,
        &stages(),
        &PipelineOptions::default(),
        &collscan_explain(),
        &|| 1_800_000_000,
    );
    for _ in 0..100 {
        if !records_mentioning("not recorded").is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !records_mentioning("typed query shape not recorded").is_empty(),
        "the failure is logged"
    );
    let all = RECORDS.lock().unwrap().join("\n");
    assert!(
        !all.contains("zebra-secret") && !all.contains("424242"),
        "{all}"
    );
}
