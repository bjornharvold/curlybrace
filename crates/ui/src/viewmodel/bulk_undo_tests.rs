use super::*;
use bson::doc;
use curlybrace_core::bulk::{BulkRunId, UndoPreview, UndoPreviewDoc};
use curlybrace_core::error::CoreError;
use curlybrace_core::snapshot::UndoReport;

fn run() -> BulkRunId {
    BulkRunId(uuid::Uuid::new_v4())
}

fn preview(documents: u64) -> UndoPreview {
    UndoPreview {
        documents,
        docs: vec![
            UndoPreviewDoc {
                original: doc! { "_id": 1, "v": 1 },
                current: Some(doc! { "_id": 1, "v": 2 }),
            },
            UndoPreviewDoc {
                original: doc! { "_id": 2 },
                current: None,
            },
        ],
    }
}

fn ready() -> UndoFlow {
    UndoFlow::open(run(), "orders".into()).with_preview(UndoPreviewState::Ready(preview(2)))
}

#[test]
fn it_opens_waiting_for_its_preview_and_cannot_run() {
    let flow = UndoFlow::open(run(), "orders".into());
    assert_eq!(flow.step, UndoStep::Confirm);
    assert_eq!(flow.preview, UndoPreviewState::Pending);
    assert!(!flow.can_run());
}

#[test]
fn running_needs_the_preview_and_the_collection_name_typed() {
    let flow = ready();
    assert!(!flow.can_run(), "nothing typed yet");
    assert!(!flow.clone().with_typed("order".into()).can_run());
    assert!(flow.clone().with_typed("orders".into()).can_run());
}

#[test]
fn a_failed_preview_never_offers_the_run() {
    let flow = UndoFlow::open(run(), "orders".into())
        .with_preview(UndoPreviewState::Failed(CoreError::SnapshotUnreadable))
        .with_typed("orders".into());
    assert!(!flow.can_run());
}

#[test]
fn a_new_preview_clears_what_was_typed() {
    let flow = ready().with_typed("orders".into());
    let flow = flow.with_preview(UndoPreviewState::Ready(preview(3)));
    assert_eq!(flow.typed, "");
}

#[test]
fn the_preview_is_shown_as_text_cut_once() {
    let flow = ready();
    assert_eq!(flow.preview_docs().len(), 2);
    let first = &flow.preview_docs()[0];
    assert!(first.original.contains("\"v\":1"));
    assert!(
        first
            .current
            .as_deref()
            .is_some_and(|c| c.contains("\"v\":2"))
    );
    assert_eq!(flow.preview_docs()[1].current, None, "gone now");
}

#[test]
fn running_starts_only_when_it_can_and_then_locks_the_inputs() {
    let flow = ready();
    assert_eq!(flow.clone().running().step, UndoStep::Confirm, "not typed");
    let typed = flow.with_typed("orders".into()).running();
    assert_eq!(typed.step, UndoStep::Running);
    assert_eq!(typed.clone().with_typed("x".into()).typed, "orders");
    assert_eq!(
        typed.with_preview(UndoPreviewState::Pending).preview,
        UndoPreviewState::Ready(preview(2))
    );
}

#[test]
fn only_a_running_flow_takes_a_report() {
    let report = Ok(UndoReport {
        restored: 2,
        ..UndoReport::default()
    });
    assert_eq!(
        ready().reported(report.clone()).step,
        UndoStep::Confirm,
        "not running"
    );
    let done = ready()
        .with_typed("orders".into())
        .running()
        .reported(report.clone());
    assert_eq!(done.step, UndoStep::Report);
    assert_eq!(done.report, Some(report));
}

#[test]
fn a_failed_undo_can_be_retried_from_its_report() {
    let failed = ready()
        .with_typed("orders".into())
        .running()
        .reported(Err(CoreError::WriteOutcomeUnknown));
    let again = failed.retried();
    assert_eq!(again.step, UndoStep::Confirm);
    assert_eq!(again.typed, "", "the confirmation is given again");
    assert_eq!(again.report, None);
    assert!(matches!(again.preview, UndoPreviewState::Pending));
}
