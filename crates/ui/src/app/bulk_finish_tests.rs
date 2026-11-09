//! The finish of a bulk run, cancel and close, and failed bulk commands.
use super::bulk_test_support::*;

#[test]
fn finished_unknown_shows_unknown_report_and_requeries() {
    let (s, id, run) = running_delete();
    let unknown = BulkOutcome::Unknown { cancelled: false };
    let (s, cmds) = ev(s, finish(id, run, Ok(unknown.clone())));
    assert_eq!(flow(&s).step, BulkStep::Report);
    assert_eq!(flow(&s).report, Some(Ok(unknown)));
    assert!(s.pending.is_none());
    assert!(matches!(
        &cmds[..],
        [Command::Find { query, .. }] if *query == s.page_query
    ));
}

#[test]
fn finished_rejected_keeps_the_reason_in_the_report_and_requeries() {
    let (s, id, run) = running_delete();
    let rejected = BulkOutcome::Rejected {
        reason: curlybrace_core::bulk::BulkFailureReason::ValidationFailed,
    };
    let (s, cmds) = ev(s, finish(id, run, Ok(rejected.clone())));
    assert_eq!(flow(&s).step, BulkStep::Report);
    assert_eq!(flow(&s).report, Some(Ok(rejected)));
    assert!(s.pending.is_none());
    assert!(cmds.iter().any(is_find));
}

#[test]
fn finished_error_is_reported_and_requeries() {
    let (s, id, run) = running_delete();
    let (s, cmds) = ev(s, finish(id, run, Err(CoreError::BulkEmptyFilter)));
    assert_eq!(flow(&s).report, Some(Err(CoreError::BulkEmptyFilter)));
    assert!(s.pending.is_none());
    assert!(cmds.iter().any(is_find));
}

#[test]
fn finished_for_unknown_run_requeries_without_a_flow() {
    let (s, id) = plain();
    let stray = BulkRunId(Uuid::new_v4());
    let (s, cmds) = ev(s, finish(id, stray, Ok(applied())));
    assert!(s.bulk.is_none());
    assert!(matches!(&cmds[..], [Command::Find { .. }]));
}

#[test]
fn finished_for_another_run_changes_no_flow_and_leaves_the_pending_write() {
    let (s, id, _) = running_delete();
    let (s, cmds) = ev(s, finish(id, BulkRunId(Uuid::new_v4()), Ok(applied())));
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert!(s.pending.is_some());
    assert!(cmds.iter().any(is_find));
}

#[test]
fn finished_on_an_unselected_collection_does_not_requery() {
    let (s, id) = plain();
    let event = Event::BulkFinished {
        session: id,
        ns: Namespace::new("shop", "other"),
        run: BulkRunId(Uuid::new_v4()),
        outcome: Ok(applied()),
    };
    let (_, cmds) = ev(s, event);
    assert!(cmds.is_empty());
}

#[test]
fn cancel_while_running_sends_bulk_cancel() {
    let (s, id, run) = running_delete();
    let (s, cmds) = act(s, BulkAction::Cancel);
    assert!(matches!(
        &cmds[..],
        [Command::BulkCancel { session, run: r }] if *session == id && *r == run
    ));
    assert_eq!(flow(&s).step, BulkStep::Running);
}

#[test]
fn close_while_running_does_nothing() {
    let (s, _, _) = running_delete();
    let (s, cmds) = act(s, BulkAction::Close);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Running);
}

#[test]
fn escape_and_close_cancel_from_every_step_except_running() {
    for action in [BulkAction::Cancel, BulkAction::Close] {
        let (define, _) = defining(BulkKind::Delete, config(false, false));
        let (preview, _) = previewing(BulkKind::Delete, config(false, false));
        let (confirm, _) = confirming(BulkKind::Delete, config(false, false));
        let (running, id, run) = running_delete();
        let (report, _) = ev(running, finish(id, run, Ok(applied())));
        for s in [define, preview, confirm, report] {
            let (s, cmds) = act(s, action.clone());
            assert!(s.bulk.is_none(), "{action:?} must close the flow");
            assert!(cmds.is_empty());
        }
    }
}

#[test]
fn closing_drops_the_outstanding_preview_id() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    let request = preview_id(&cmds);
    let (s, _) = act(s, BulkAction::Close);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Delete, request));
    assert!(s.bulk.is_none());
}

#[test]
fn cancel_unavailable_is_recorded_and_run_continues() {
    let (s, id, run) = running_delete();
    let (s, cmds) = ev(
        s,
        Event::BulkCancelResult {
            session: id,
            run,
            outcome: Ok(()),
        },
    );
    assert!(!flow(&s).cancel_unavailable && cmds.is_empty());
    let (s, cmds) = ev(
        s,
        Event::BulkCancelResult {
            session: id,
            run,
            outcome: Err(CoreError::Unreachable),
        },
    );
    assert!(flow(&s).cancel_unavailable);
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert!(cmds.is_empty());
}

#[test]
fn a_cancel_result_for_another_run_is_ignored() {
    let (s, id, _) = running_delete();
    let (s, _) = ev(
        s,
        Event::BulkCancelResult {
            session: id,
            run: BulkRunId(Uuid::new_v4()),
            outcome: Err(CoreError::Unreachable),
        },
    );
    assert!(!flow(&s).cancel_unavailable);
}

#[test]
fn failed_read_only_closes_the_flow_with_the_banner() {
    let (s, id, _) = running_delete();
    let (s, cmds) = ev(s, failed(id, "bulk_delete", CoreError::ReadOnly));
    assert!(s.bulk.is_none());
    assert!(s.pending.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(cmds.is_empty());
}

#[test]
fn another_failed_write_is_reported_in_the_failure_banner_and_closes_the_flow() {
    let (s, id, _) = running_delete();
    let (s, _) = ev(s, failed(id, "bulk_delete", CoreError::BulkScopeMismatch));
    assert!(s.bulk.is_none());
    assert!(s.pending.is_none());
    assert_eq!(
        s.banner,
        Some(Banner::Failure {
            context: "bulk_delete",
            error: CoreError::BulkScopeMismatch
        })
    );
}

#[test]
fn a_failed_preview_command_marks_the_preview_failed_and_keeps_the_flow() {
    // A preview has no request id, so the runtime reports its refusal as `Failed`.
    let (s, id) = defining(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::Next);
    let (s, _) = ev(
        s,
        failed(id, "bulk_preview", CoreError::Database("x".into())),
    );
    assert_eq!(
        flow(&s).preview,
        PreviewState::Failed(CoreError::Database("x".into()))
    );
    assert!(s.banner.is_none());
}

#[test]
fn a_late_refused_write_does_not_close_a_flow_that_is_not_running() {
    // A flow in Confirm (nothing in flight): a `Failed` of an older write is not about it.
    for (context, error) in [
        ("bulk_delete", CoreError::BulkScopeMismatch),
        ("bulk_update", CoreError::ReadOnly),
    ] {
        let (s, id) = confirming(BulkKind::Delete, config(false, false));
        let before = s.bulk.clone();
        let (after, _) = ev(s, failed(id, context, error));
        assert_eq!(after.bulk, before);
        assert!(after.banner.is_none());
    }
}

#[test]
fn a_refused_write_with_no_write_in_flight_leaves_the_flow_alone() {
    let (mut s, id, _) = running_delete();
    s.pending = None;
    let before = s.bulk.clone();
    let (after, _) = ev(s, failed(id, "bulk_delete", CoreError::BulkScopeMismatch));
    assert_eq!(after.bulk, before);
    assert!(after.banner.is_none());
}

#[test]
fn a_failed_cancel_only_raises_the_banner_and_is_not_a_data_failure() {
    let (s, id, _) = running_delete();
    let (s, _) = ev(
        s,
        failed(id, "bulk_cancel", CoreError::Database("x".into())),
    );
    assert_eq!(flow(&s).step, BulkStep::Running);
    let banner = s.banner.as_ref().expect("a banner");
    assert!(matches!(banner, Banner::Failure { .. }));
    assert!(!banner.is_data_failure());
}
