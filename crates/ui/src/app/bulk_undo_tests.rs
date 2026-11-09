//! Undoing a bulk run from its report: the policy the write is sent with, the undo's preview and
//! typed confirmation, its guards, and the "run without undo" and "keep nothing" ways out.
use super::bulk_test_support::*;
use crate::viewmodel::{UndoPreviewState, UndoStep};
use curlybrace_core::bulk::{UndoOffer, UndoPolicy, UndoPreview, UndoPreviewDoc, UndoStatus};
use curlybrace_core::snapshot::UndoReport;

fn offered() -> UndoStatus {
    UndoStatus::Offered(UndoOffer {
        documents: 5,
        uncovered: 0,
    })
}

fn finish_with(id: Uuid, run: BulkRunId, undo: UndoStatus) -> Event {
    Event::BulkFinished {
        session: id,
        ns: ns(),
        run,
        outcome: Ok(applied()),
        undo,
    }
}

/// A delete that finished with an undo copy on offer.
fn reported_with_copy() -> (AppState, Uuid, BulkRunId) {
    let (s, id, run) = running_delete();
    let (s, _) = ev(s, finish_with(id, run, offered()));
    (s, id, run)
}

fn undo_preview_event(id: Uuid, run: BulkRunId, request_id: u64) -> Event {
    Event::BulkUndoPreview {
        session: id,
        run,
        request_id,
        outcome: Ok(UndoPreview {
            documents: 5,
            docs: vec![UndoPreviewDoc {
                original: doc! { "_id": 1 },
                current: None,
            }],
        }),
    }
}

fn undo_request(cmds: &[Command]) -> u64 {
    match cmds {
        [Command::BulkUndoPreview { request_id, .. }] => *request_id,
        other => panic!("expected one BulkUndoPreview, got {other:?}"),
    }
}

/// The undo opened, previewed and its collection name typed: ready to run.
fn undo_ready() -> (AppState, Uuid, BulkRunId) {
    let (s, id, run) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::OpenUndo);
    let (s, _) = ev(s, undo_preview_event(id, run, undo_request(&cmds)));
    let (s, _) = act(s, BulkAction::SetUndoTyped("orders".into()));
    (s, id, run)
}

fn undo_of(s: &AppState) -> &crate::viewmodel::UndoFlow {
    flow(s).undo_flow.as_ref().expect("an undo is open")
}

#[test]
fn a_write_asks_for_an_undo_copy_by_default() {
    let (s, _) = ready_delete(config(false, false));
    let (_, cmds) = act(s, BulkAction::Run);
    assert!(matches!(
        &cmds[..],
        [Command::BulkDelete {
            undo: UndoPolicy::Snapshot,
            ..
        }]
    ));
}

#[test]
fn the_report_keeps_what_the_run_says_about_undoing_it() {
    let (s, _, _) = reported_with_copy();
    assert_eq!(flow(&s).undo, offered());
}

#[test]
fn opening_the_undo_asks_what_it_would_restore() {
    let (s, id, run) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::OpenUndo);
    assert!(matches!(
        &cmds[..],
        [Command::BulkUndoPreview { session, run: r, .. }] if *session == id && *r == run
    ));
    assert_eq!(undo_of(&s).preview, UndoPreviewState::Pending);
}

#[test]
fn nothing_opens_when_the_run_offers_no_undo() {
    let (s, id, run) = running_delete();
    let (s, _) = ev(s, finish_with(id, run, UndoStatus::Lost));
    let (s, cmds) = act(s, BulkAction::OpenUndo);
    assert!(cmds.is_empty());
    assert!(flow(&s).undo_flow.is_none());
}

#[test]
fn only_the_awaited_preview_answer_is_taken() {
    let (s, id, run) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::OpenUndo);
    let awaited = undo_request(&cmds);
    let (s, _) = ev(s, undo_preview_event(id, run, awaited + 99));
    assert_eq!(undo_of(&s).preview, UndoPreviewState::Pending, "stale id");
    let (s, _) = ev(
        s,
        undo_preview_event(id, BulkRunId(Uuid::new_v4()), awaited),
    );
    assert_eq!(undo_of(&s).preview, UndoPreviewState::Pending, "other run");
    let (s, _) = ev(s, undo_preview_event(id, run, awaited));
    assert!(matches!(undo_of(&s).preview, UndoPreviewState::Ready(_)));
}

#[test]
fn the_undo_needs_the_typed_collection_name_before_it_is_sent() {
    let (s, id, run) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::OpenUndo);
    let (s, _) = ev(s, undo_preview_event(id, run, undo_request(&cmds)));
    let (s, cmds) = act(s, BulkAction::RunUndo);
    assert!(cmds.is_empty(), "nothing typed");
    let (s, _) = act(s, BulkAction::SetUndoTyped("order".into()));
    let (s, cmds) = act(s, BulkAction::RunUndo);
    assert!(cmds.is_empty(), "the name is not complete");
    let (s, _) = act(s, BulkAction::SetUndoTyped("orders".into()));
    let (s, cmds) = act(s, BulkAction::RunUndo);
    assert!(matches!(&cmds[..], [Command::BulkUndo { run: r, .. }] if *r == run));
    assert_eq!(undo_of(&s).step, UndoStep::Running);
    assert!(s.pending.is_some(), "a write is in flight");
}

#[test]
fn a_running_undo_cannot_be_closed_or_left() {
    let (s, _, _) = undo_ready();
    let (mut s, _) = act(s, BulkAction::RunUndo);
    let running = s.bulk.clone();
    for action in [BulkAction::Close, BulkAction::Cancel, BulkAction::CloseUndo] {
        let (next, cmds) = act(s, action);
        assert!(cmds.is_empty());
        assert_eq!(next.bulk, running);
        s = next;
    }
}

#[test]
fn the_undo_report_lands_and_the_page_is_read_again() {
    let (s, id, run) = undo_ready();
    let (s, _) = act(s, BulkAction::RunUndo);
    let report = UndoReport {
        restored: 5,
        ..UndoReport::default()
    };
    let (s, cmds) = ev(
        s,
        Event::BulkUndoFinished {
            session: id,
            run,
            outcome: Ok(report),
        },
    );
    assert_eq!(undo_of(&s).step, UndoStep::Report);
    assert_eq!(undo_of(&s).report, Some(Ok(report)));
    assert!(s.pending.is_none());
    assert!(cmds.iter().any(is_find));
}

#[test]
fn a_refused_undo_is_told_in_its_report_where_it_can_be_retried() {
    let (s, id, _) = undo_ready();
    let (s, _) = act(s, BulkAction::RunUndo);
    let (s, _) = ev(s, failed(id, "bulk_undo", CoreError::ReadOnly));
    assert_eq!(undo_of(&s).step, UndoStep::Report);
    assert_eq!(undo_of(&s).report, Some(Err(CoreError::ReadOnly)));
    assert!(s.pending.is_none());
    let (s, cmds) = act(s, BulkAction::RetryUndo);
    assert!(matches!(&cmds[..], [Command::BulkUndoPreview { .. }]));
    assert_eq!(undo_of(&s).step, UndoStep::Confirm);
    assert_eq!(undo_of(&s).typed, "");
}

#[test]
fn a_failed_undo_preview_is_shown_and_blocks_the_run() {
    let (s, id, _) = reported_with_copy();
    let (s, _) = act(s, BulkAction::OpenUndo);
    let (s, _) = ev(
        s,
        failed(id, "bulk_undo_preview", CoreError::SnapshotOtherConnection),
    );
    assert_eq!(
        undo_of(&s).preview,
        UndoPreviewState::Failed(CoreError::SnapshotOtherConnection)
    );
    let (s, _) = act(s, BulkAction::SetUndoTyped("orders".into()));
    assert!(act(s, BulkAction::RunUndo).1.is_empty());
}

#[test]
fn a_connection_locked_read_only_since_refuses_the_undo_and_ends_the_flow() {
    let (mut s, _, _) = undo_ready();
    s.read_only = true;
    let (s, cmds) = act(s, BulkAction::RunUndo);
    assert!(cmds.is_empty());
    assert!(s.bulk.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
}

#[test]
fn going_back_from_the_undo_shows_the_report_again() {
    let (s, _, _) = undo_ready();
    let (s, _) = act(s, BulkAction::CloseUndo);
    assert!(flow(&s).undo_flow.is_none());
    assert_eq!(flow(&s).step, BulkStep::Report);
}

#[test]
fn keeping_nothing_deletes_the_copy_and_closes_the_dialog() {
    let (s, _, run) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::DiscardCopy);
    assert!(s.bulk.is_none());
    assert!(matches!(&cmds[..], [Command::SnapshotDelete { run: r, .. }] if *r == run));
}

#[test]
fn closing_the_report_keeps_the_copy() {
    let (s, _, _) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::Close);
    assert!(s.bulk.is_none());
    assert!(cmds.is_empty(), "no command deletes the copy");
}

#[test]
fn a_refused_copy_can_be_overridden_but_only_with_the_name_typed_again() {
    let (s, id, run) = running_delete();
    let (s, _) = ev(s, finish(id, run, Err(CoreError::SnapshotTooLarge)));
    assert!(flow(&s).snapshot_refused());
    let (s, cmds) = act(s, BulkAction::RunWithoutUndo);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Confirm);
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty(), "name not typed");
    let (s, _) = act(s, BulkAction::SetTyped("orders".into()));
    let (_, cmds) = act(s, BulkAction::Run);
    assert!(matches!(
        &cmds[..],
        [Command::BulkDelete {
            undo: UndoPolicy::None,
            ..
        }]
    ));
}

#[test]
fn running_without_undo_is_only_possible_after_a_refused_copy() {
    let (s, id, run) = running_delete();
    let (s, _) = ev(s, finish(id, run, Err(CoreError::BulkCancelled)));
    let (s, _) = act(s, BulkAction::RunWithoutUndo);
    assert_eq!(flow(&s).step, BulkStep::Report, "unchanged");
}

#[test]
fn opening_a_flow_records_where_the_undo_copy_will_be_kept() {
    let (s, _) = plain();
    let (s, _) = act(s, BulkAction::Open(BulkKind::Delete));
    let want =
        curlybrace_core::runtime::default_snapshot_dir().map(|dir| dir.display().to_string());
    assert_eq!(flow(&s).undo_folder, want);
}
