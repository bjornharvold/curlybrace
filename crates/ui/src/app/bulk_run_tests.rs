//! Running the bulk write through the guard, and the session changing under the flow.
use super::bulk_test_support::*;

#[test]
fn run_without_typed_confirmation_sends_nothing() {
    let (s, _) = confirming(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Confirm);
    assert!(s.pending.is_none());
}

#[test]
fn run_is_a_noop_when_can_run_is_false() {
    let (s, _) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Define);
}

#[test]
fn run_with_confirmation_sends_bulk_delete_once() {
    let (s, id) = ready_delete(config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    let [
        Command::BulkDelete {
            session,
            ns: n,
            request,
            run,
        },
    ] = &cmds[..]
    else {
        panic!("expected one BulkDelete, got {cmds:?}");
    };
    assert_eq!((*session, n), (id, &ns()));
    assert_eq!(request.filter, doc! { "status": "old" });
    assert_eq!(request.scope, MatchScope::Filtered);
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert_eq!(flow(&s).run, Some(*run));
    assert_eq!(s.pending.as_ref().map(|p| &p.ns), Some(&ns()));
    let (_, again) = act(s, BulkAction::Run);
    assert!(again.is_empty());
}

#[test]
fn run_sends_bulk_update_with_the_parsed_update() {
    let (s, _) = confirming(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::Back);
    let (s, _) = act(s, BulkAction::Back);
    let (s, _) = act(s, BulkAction::SetUpsert(true));
    let id = s.bulk.as_ref().unwrap().session;
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Update, preview_id(&cmds)));
    let (s, _) = act(s, BulkAction::Next);
    let (s, cmds) = act(s, BulkAction::Run);
    let [Command::BulkUpdate { request, run, .. }] = &cmds[..] else {
        panic!("expected one BulkUpdate, got {cmds:?}");
    };
    assert!(request.upsert);
    assert_eq!(request.filter, doc! { "status": "old" });
    assert_eq!(flow(&s).run, Some(*run));
}

#[test]
fn run_on_production_sends_without_a_second_warning() {
    let (s, _) = ready_delete(config(true, false));
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(s.write_warning.is_none());
    assert!(matches!(&cmds[..], [Command::BulkDelete { .. }]));
}

#[test]
fn run_is_refused_when_the_connection_became_read_only() {
    let (mut s, id) = ready_delete(config(false, false));
    s.conn.connections.iter_mut().for_each(|c| {
        if c.id == id {
            c.read_only = true;
        }
    });
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(s.bulk.is_none(), "the refused write ends the flow");
    assert!(s.pending.is_none());
}

#[test]
fn run_after_the_selected_collection_changed_closes_the_flow() {
    let (mut s, _) = ready_delete(config(false, false));
    s.sidebar = s.sidebar.selecting(Namespace::new("shop", "other"));
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert!(s.bulk.is_none());
    assert!(s.pending.is_none());
}

#[test]
fn run_is_ignored_while_another_write_is_in_flight() {
    let (mut s, _) = ready_delete(config(false, false));
    s.pending = Some(Pending {
        ns: ns(),
        editor: None,
    });
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Confirm);
}

#[test]
fn epoch_change_closes_the_flow_and_run_sends_nothing() {
    let (mut s, _) = ready_delete(config(false, false));
    s.latest_requested_epoch += 1;
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert!(s.bulk.is_none());
    assert!(s.pending.is_none());
}

#[test]
fn a_lost_connection_closes_an_idle_flow() {
    let (s, id) = confirming(BulkKind::Delete, config(false, false));
    let (s, _) = ev(s, failed(id, "find", CoreError::Unreachable));
    assert!(s.bulk.is_none());
    assert_eq!(s.status_of(id), Some(ConnStatus::Failed));
}

#[test]
fn disconnect_closes_the_flow_and_a_reconnect_does_not_bring_it_back() {
    let (s, id) = confirming(BulkKind::Delete, config(false, false));
    let (s, _) = update(s, Input::Disconnect(id));
    assert!(s.bulk.is_none());
    assert_eq!(s.bulk_requests, 2, "the counter is not reset");
}

#[test]
fn a_reconnect_closes_the_flow() {
    let (s, id) = confirming(BulkKind::Delete, config(false, false));
    let (s, _) = ev(s, failed(id, "find", CoreError::Unreachable));
    let (s, _) = update(s, Input::Connect(id));
    assert!(s.bulk.is_none());
}

#[test]
fn session_loss_during_running_still_reports_the_finish() {
    let (s, id, run) = running_delete();
    let (s, _) = ev(s, failed(id, "find", CoreError::Unreachable));
    assert_eq!(s.status_of(id), Some(ConnStatus::Failed));
    assert_eq!(flow(&s).step, BulkStep::Running, "the run is still ours");
    let unknown = BulkOutcome::Unknown { cancelled: false };
    let (s, cmds) = ev(s, finish(id, run, Ok(unknown.clone())));
    assert_eq!(flow(&s).step, BulkStep::Report);
    assert_eq!(flow(&s).report, Some(Ok(unknown)));
    assert!(cmds.is_empty(), "a lost session has nothing to re-query");
}

#[test]
fn a_flow_closed_with_its_session_gets_no_report_and_no_crash() {
    let (s, id, run) = running_delete();
    let (s, _) = update(s, Input::Disconnect(id));
    let (s, cmds) = ev(s, finish(id, run, Ok(applied())));
    assert!(s.bulk.is_none());
    assert!(cmds.is_empty());
}
