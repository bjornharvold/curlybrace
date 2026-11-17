//! The aggregate commands carry the person's options and each handler sends what its operation
//! sends (`PipelineOptions::applied`): the in-memory store records what reached it.

use std::time::Duration;

use bson::{Bson, Document, doc};

use super::aggregate_tests::{connected_to, next};
use super::*;
use crate::explain::ExplainVerbosity;
use crate::pipeline::{
    AppliedOptions, CaseFirst, CollationOption, HintOption, Operation, PipelineOptions,
    ReadPreferenceMode, Strength,
};
use crate::store::{Namespace, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::mock_namespace;

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: Some(Strength::Secondary),
            case_level: None,
            case_first: Some(CaseFirst::Upper),
            numeric_ordering: Some(true),
            alternate: None,
        }),
        hint: Some(HintOption::Name("by_status".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(2_000),
        comment: Some("nightly".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    }
}

fn pipeline() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "open" } },
        doc! { "$limit": 20 },
    ]
}

fn out_pipeline() -> Vec<Document> {
    vec![doc! { "$match": {} }, doc! { "$out": "dest" }]
}

fn preview(session: SessionId, options: PipelineOptions) -> Command {
    Command::AggregatePreview {
        session,
        request: 1,
        ns: mock_namespace(),
        pipeline: pipeline(),
        options,
    }
}

fn count(session: SessionId, options: PipelineOptions) -> Command {
    Command::AggregateCount {
        session,
        request: 1,
        ns: mock_namespace(),
        stage_index: 0,
        pipeline: pipeline(),
        options,
    }
}

fn explain(session: SessionId, options: PipelineOptions) -> Command {
    Command::AggregateExplain {
        session,
        request: 1,
        ns: mock_namespace(),
        stages: pipeline(),
        verbosity: ExplainVerbosity::ExecutionStats,
        options,
    }
}

fn run_command(session: SessionId, options: PipelineOptions) -> Command {
    Command::AggregateRun {
        session,
        ns: mock_namespace(),
        pipeline: out_pipeline(),
        run: 1,
        options,
    }
}

fn store() -> Arc<InMemoryStore> {
    let target = Namespace::new(mock_namespace().db, "dest");
    Arc::new(
        InMemoryStore::new()
            .with_aggregate(vec![doc! { "n": 3_i32 }])
            .with_aggregate_explain(doc! { "ok": 1 })
            .with_docs(&target, vec![doc! { "_id": 1 }]),
    )
}

#[tokio::test]
async fn a_preview_sends_the_options_it_was_given() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(preview(session, every_option())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregatePreviewed { outcome: Ok(_), .. }
    ));
    let sent = store.received_options();
    assert_eq!(
        sent,
        vec![(
            Operation::Preview,
            every_option().applied(Operation::Preview)
        )]
    );
    let applied = &sent[0].1;
    assert_eq!(applied.max_time, Duration::from_millis(2_000));
    assert_eq!(applied.allow_disk_use, Some(false));
    assert_eq!(applied.hint, Some(HintOption::Name("by_status".into())));
    assert_eq!(applied.comment.as_deref(), Some("nightly"));
    assert_eq!(
        applied.read_preference,
        Some(ReadPreferenceMode::SecondaryPreferred)
    );
}

#[tokio::test]
async fn the_default_options_preview_as_before() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(preview(session, PipelineOptions::default()))
        .unwrap();
    next(&mut h).await;
    let before = AppliedOptions::bounded(
        Operation::Preview,
        Duration::from_millis(crate::pipeline::PREVIEW_MAX_TIME_MS),
    );
    assert_eq!(store.received_options(), vec![(Operation::Preview, before)]);
}

#[tokio::test]
async fn a_count_sends_the_options_without_disk_use() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(count(session, every_option())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateCounted { outcome: Ok(3), .. }
    ));
    let sent = store.received_options();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, every_option().applied(Operation::Count));
    assert_eq!(sent[0].1.allow_disk_use, Some(false));
    assert_eq!(sent[0].1.max_time, Duration::from_millis(2_000));
    assert_eq!(
        sent[0].1.collation.as_ref().map(|c| c.locale.as_str()),
        Some("en")
    );
}

#[tokio::test]
async fn an_explain_sends_disk_use_and_read_preference_as_set() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(explain(session, every_option())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateExplained { outcome: Ok(_), .. }
    ));
    let sent = store.received_options();
    assert_eq!(
        sent,
        vec![(
            Operation::Explain,
            every_option().applied(Operation::Explain)
        )]
    );
    assert_eq!(sent[0].1.allow_disk_use, Some(true));
    assert_eq!(
        sent[0].1.read_preference,
        Some(ReadPreferenceMode::SecondaryPreferred)
    );
}

#[tokio::test]
async fn a_run_never_sends_the_read_preference() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, every_option())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregateRan { outcome: Ok(_), .. }
    ));
    let sent = store.received_options();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, Operation::Run);
    assert_eq!(sent[0].1.read_preference, None);
}

#[tokio::test]
async fn a_run_carries_collation_hint_and_a_lower_time() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, every_option())).unwrap();
    next(&mut h).await;
    let applied = store.received_options().remove(0).1;
    assert_eq!(applied.max_time, Duration::from_millis(2_000));
    assert_eq!(applied.hint, Some(HintOption::Name("by_status".into())));
    assert!(applied.collation.is_some());
    assert_eq!(applied.allow_disk_use, Some(true));
    // Without a lower value the run keeps its own 60 s.
    let (mut h, session) = connected_to(store.clone(), false, RuntimeConfig::default()).await;
    h.tx.send(run_command(session, PipelineOptions::default()))
        .unwrap();
    next(&mut h).await;
    let defaults = store.received_options().remove(1).1;
    assert_eq!(defaults.max_time, Duration::from_secs(60));
}

#[tokio::test]
async fn the_confirmed_run_is_never_raised_above_sixty_seconds() {
    let options = PipelineOptions {
        max_time_ms: Some(60_000),
        ..PipelineOptions::default()
    };
    assert_eq!(
        options.applied(Operation::Run).max_time,
        Duration::from_secs(60)
    );
}

#[tokio::test]
async fn a_read_only_session_previews_with_options() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), true, RuntimeConfig::default()).await;
    h.tx.send(preview(session, every_option())).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::AggregatePreviewed { outcome: Ok(_), .. }
    ));
    assert_eq!(store.received_options().len(), 1);
}

#[tokio::test]
async fn a_read_only_session_still_refuses_a_run_with_options() {
    let store = store();
    let (mut h, session) = connected_to(store.clone(), true, RuntimeConfig::default()).await;
    let command = run_command(session, every_option());
    assert!(command.is_write());
    h.tx.send(command).unwrap();
    let event = next(&mut h).await;
    assert!(
        matches!(
            &event,
            Event::Failed {
                error: CoreError::ReadOnly,
                ..
            } | Event::RequestFailed {
                error: CoreError::ReadOnly,
                ..
            }
        ),
        "{event:?}"
    );
    assert!(store.received_options().is_empty());
    assert!(store.written_pipelines().is_empty());
}

/// A store whose preview fails at once with `error`, and whose ping fails: a lost connection
/// would show as a failed command, anything else as the outcome.
struct FailingPreview(CoreError);

#[async_trait::async_trait]
impl DocumentStore for FailingPreview {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<crate::store::Page> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn ping(&self) -> Result<()> {
        Err(CoreError::Unreachable)
    }
    async fn aggregate_preview(
        &self,
        _ns: &Namespace,
        _pipeline: Vec<Document>,
        _options: &AppliedOptions,
    ) -> Result<Vec<Document>> {
        Err(self.0.clone())
    }
}

#[tokio::test]
async fn an_option_failure_is_the_previews_outcome_not_a_lost_connection() {
    for error in [
        CoreError::ReadPreferenceUnavailable,
        CoreError::HintNotFound,
        CoreError::CollationRejected,
    ] {
        let (mut h, session) = connected_to(
            Arc::new(FailingPreview(error.clone())),
            false,
            RuntimeConfig::default(),
        )
        .await;
        h.tx.send(preview(session, every_option())).unwrap();
        match next(&mut h).await {
            Event::AggregatePreviewed { outcome, .. } => assert_eq!(outcome, Err(error)),
            other => panic!("expected the preview's outcome, got {other:?}"),
        }
    }
}
