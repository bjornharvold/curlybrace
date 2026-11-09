//! A reconnect keeps a still-running bulk write's flow so the late finish is reported on it.
use super::bulk_test_support::*;
use super::reducer::Input;

fn lost_find(id: Uuid) -> Event {
    failed(id, "find", CoreError::Unreachable)
}

/// A running delete whose connection dropped and that was then reconnected.
fn reconnected_running() -> (AppState, Uuid, BulkRunId) {
    let (s, id, run) = running_delete();
    let (s, _) = ev(s, lost_find(id));
    assert_eq!(s.status, ConnStatus::Failed);
    assert_eq!(
        flow(&s).step,
        BulkStep::Running,
        "failed() keeps a running flow"
    );
    let (s, cmds) = update(s, Input::Connect(id));
    assert!(matches!(&cmds[..], [Command::Connect(_)]));
    (s, id, run)
}

#[test]
fn a_running_flow_survives_a_reconnect() {
    let (s, _, run) = reconnected_running();
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert_eq!(flow(&s).run, Some(run));
}

#[test]
fn the_late_unknown_finish_is_reported_after_a_reconnect() {
    let (s, id, run) = reconnected_running();
    let unknown = BulkOutcome::Unknown { cancelled: false };
    let (s, _) = ev(s, finish(id, run, Ok(unknown.clone())));
    assert_eq!(flow(&s).step, BulkStep::Report);
    assert_eq!(flow(&s).report, Some(Ok(unknown)));
}

#[test]
fn the_late_rejected_finish_is_reported_after_a_reconnect() {
    let (s, id, run) = reconnected_running();
    let rejected = BulkOutcome::Rejected {
        reason: curlybrace_core::bulk::BulkFailureReason::ValidationFailed,
    };
    let (s, _) = ev(s, finish(id, run, Ok(rejected.clone())));
    assert_eq!(flow(&s).step, BulkStep::Report);
    assert_eq!(flow(&s).report, Some(Ok(rejected)));
}

#[test]
fn a_flow_that_is_not_running_is_dropped_by_a_reconnect() {
    let (mut s, id) = confirming(BulkKind::Delete, config(false, false));
    s.status = ConnStatus::Failed;
    let (s, _) = update(s, Input::Connect(id));
    assert!(s.bulk.is_none());
}

#[test]
fn stale_pending_after_a_reconnect_does_not_wedge_the_next_write() {
    let (s, id, run) = reconnected_running();
    assert!(s.pending.is_none(), "the reconnect starts a fresh view");
    let (s, _) = ev(s, finish(id, run, Ok(applied())));
    let (s, _) = act(s, BulkAction::Close);
    assert!(s.bulk.is_none() && s.pending.is_none());
}
