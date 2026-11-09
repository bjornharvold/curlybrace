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
    let timeout = CoreError::Database("timeout".into());
    let (s, _) = ev(s, failed(id, "bulk_undo", timeout.clone()));
    assert_eq!(undo_of(&s).step, UndoStep::Report);
    assert_eq!(undo_of(&s).report, Some(Err(timeout)));
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
fn without_a_snapshot_folder_the_write_is_the_old_one_that_cannot_be_undone() {
    let (mut s, id) = browsing(config(false, false));
    s.snapshot_dir = None;
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, count_answer(id, count_id(&cmds), Some(5)));
    assert_eq!(flow(&s).undo_policy, UndoPolicy::None);
    assert_eq!(flow(&s).undo_folder, None);
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Delete, preview_id(&cmds)));
    let (s, _) = act(s, BulkAction::Next);
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
fn the_dialog_names_the_folder_the_runtime_was_given() {
    let (s, _) = plain();
    let (s, _) = act(s, BulkAction::Open(BulkKind::Delete));
    assert_eq!(flow(&s).undo_folder.as_deref(), Some("/snapshots"));
}

#[test]
fn a_running_undo_survives_a_lost_connection_and_a_reconnect() {
    let (s, id, _) = undo_ready();
    let (s, _) = act(s, BulkAction::RunUndo);
    assert_eq!(undo_of(&s).step, UndoStep::Running);
    // `keep_running` is what a lost connection and a reconnect pass.
    let kept = s.bulk_dropped_for(id, true);
    assert!(
        kept.bulk.is_some(),
        "its report must still have a place to land"
    );
    // Closing the session drops it.
    assert!(kept.bulk_dropped_for(id, false).bulk.is_none());
}

#[test]
fn an_undo_that_is_not_running_is_dropped_with_a_lost_connection() {
    let (s, id, _) = undo_ready();
    assert!(s.bulk_dropped_for(id, true).bulk.is_none());
}

#[test]
fn leaving_the_undo_forgets_the_preview_it_awaited() {
    let (s, id, run) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::OpenUndo);
    let awaited = undo_request(&cmds);
    let (s, _) = act(s, BulkAction::CloseUndo);
    assert_eq!(s.bulk.as_ref().unwrap().undo_request, None);
    // The late answer of the abandoned preview is not taken for anything.
    let (s, _) = ev(s, undo_preview_event(id, run, awaited));
    assert!(flow(&s).undo_flow.is_none());
    let timeout = CoreError::Database("timeout".into());
    let (s, _) = ev(s, failed(id, "bulk_undo_preview", timeout));
    assert!(flow(&s).undo_flow.is_none());
}

#[test]
fn a_copy_that_is_still_there_after_deleting_it_is_told() {
    let (s, _, run) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::DiscardCopy);
    let request = match &cmds[..] {
        [Command::SnapshotDelete { request, .. }] => *request,
        other => panic!("expected SnapshotDelete, got {other:?}"),
    };
    let still_listed = curlybrace_core::snapshot::SnapshotInfo {
        run,
        ns: ns(),
        op: curlybrace_core::snapshot::SnapshotOp::Delete,
        created_unix: 0,
        bytes: 10,
    };
    let (s, _) = ev(
        s,
        Event::Snapshots {
            request,
            snapshots: vec![still_listed],
        },
    );
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "snapshot_delete",
            ..
        })
    ));
    assert!(s.discarding_copy.is_none());
}

#[test]
fn a_deleted_copy_raises_no_banner_and_another_listing_is_ignored() {
    let (s, _, _) = reported_with_copy();
    let (s, cmds) = act(s, BulkAction::DiscardCopy);
    let request = match &cmds[..] {
        [Command::SnapshotDelete { request, .. }] => *request,
        other => panic!("expected SnapshotDelete, got {other:?}"),
    };
    let (s, _) = ev(
        s,
        Event::Snapshots {
            request: request + 1,
            snapshots: vec![],
        },
    );
    assert!(s.discarding_copy.is_some(), "not the answer awaited");
    let (s, _) = ev(
        s,
        Event::Snapshots {
            request,
            snapshots: vec![],
        },
    );
    assert!(s.banner.is_none());
    assert!(s.discarding_copy.is_none());
}

#[test]
fn a_failed_preview_that_can_clear_is_read_again_from_the_confirm_step() {
    let (s, id, _) = reported_with_copy();
    let (s, _) = act(s, BulkAction::OpenUndo);
    let timeout = CoreError::Database("timeout".into());
    let (s, _) = ev(s, failed(id, "bulk_undo_preview", timeout));
    let (s, cmds) = act(s, BulkAction::RetryUndo);
    assert!(matches!(&cmds[..], [Command::BulkUndoPreview { .. }]));
    assert_eq!(undo_of(&s).preview, UndoPreviewState::Pending);
}

#[test]
fn a_failed_preview_that_cannot_clear_is_not_retried() {
    let (s, id, _) = reported_with_copy();
    let (s, _) = act(s, BulkAction::OpenUndo);
    let (s, _) = ev(
        s,
        failed(id, "bulk_undo_preview", CoreError::SnapshotUnreadable),
    );
    let (_, cmds) = act(s, BulkAction::RetryUndo);
    assert!(cmds.is_empty());
}
