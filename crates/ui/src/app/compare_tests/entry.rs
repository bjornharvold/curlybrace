//! Entry points: reloading a failed listing, "Compare to..." from the sidebar, hiding.
use super::*;
use crate::viewmodel::SidebarAction;
use curlybrace_core::store::Namespace;

#[test]
fn reload_relists_the_collections_of_the_picked_database() {
    let (_, cmds) = act(picked_state(), CompareAction::Reload(Side::Target));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Collections {
            session: 1,
            side: Side::Target,
            ..
        })]
    ));
}

#[test]
fn reload_without_a_picked_database_or_a_ready_session_does_nothing() {
    let (opened_state, _, _) = opened();
    let (_, cmds) = act(opened_state, CompareAction::Reload(Side::Source));
    assert!(cmds.is_empty());
    let (_, cmds) = act(AppState::default(), CompareAction::Reload(Side::Source));
    assert!(cmds.is_empty());
}

#[test]
fn reload_does_not_reset_the_preflight() {
    let (state, _) = act(checked_state(), CompareAction::Reload(Side::Source));
    assert_eq!(state.compare.preflight, PreflightView::Ok);
}

fn connected_state() -> (AppState, uuid::Uuid) {
    let (state, a, _) = two_connections();
    let state = AppState {
        status: ConnStatus::Connected,
        connected: Some(a),
        ..state
    };
    (state, a)
}

fn compare_to(state: AppState) -> (AppState, Vec<Command>) {
    let ns = Namespace::new("shop", "orders");
    update(state, Input::Sidebar(SidebarAction::CompareTo(ns)))
}

#[test]
fn compare_to_opens_a_session_prefills_the_source_and_shows_the_workspace() {
    let (state, a) = connected_state();
    let (state, cmds) = compare_to(state);
    assert_eq!(state.workspace, Workspace::Compare);
    assert_eq!(state.compare.connections, Some((a, a)));
    let source = &state.compare.setup.source;
    assert_eq!(source.database.as_deref(), Some("shop"));
    assert_eq!(source.collection.as_deref(), Some("orders"));
    assert!(state.compare.setup.target.database.is_none());
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Open { session: 1, .. })]
    ));
}

#[test]
fn the_prefilled_database_is_listed_once_the_session_opens() {
    let (state, _) = connected_state();
    let (state, _) = compare_to(state);
    let (_, cmds) = update(state, ev(CompareEvent::Opened { session: 1 }));
    assert!(cmds.iter().any(|c| matches!(
        c,
        Command::Compare(CompareCommand::Collections {
            side: Side::Source,
            db,
            ..
        }) if db == "shop"
    )));
    assert_eq!(
        cmds.len(),
        3,
        "two database listings and one collection listing"
    );
}

#[test]
fn compare_to_on_a_ready_session_keeps_it_and_lists_the_new_database() {
    let (state, a) = connected_state();
    let (state, _) = act(state, open_action(a, a));
    let (state, _) = update(state, ev(CompareEvent::Opened { session: 1 }));
    let (state, cmds) = compare_to(state);
    assert_eq!(state.compare.session, Some(SessionPhase::Ready(1)));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Collections {
            side: Side::Source,
            ..
        })]
    ));
    assert_eq!(state.workspace, Workspace::Compare);
}

#[test]
fn compare_to_without_a_connection_only_reports_it() {
    let (state, cmds) = compare_to(AppState::default());
    assert!(cmds.is_empty());
    assert_eq!(state.workspace, Workspace::Browse);
    assert_eq!(state.banner, Some(Banner::NotConnected));
}

#[test]
fn hiding_the_workspace_keeps_the_session_and_the_form() {
    let (state, _) = act(picked_state(), CompareAction::Hide);
    assert_eq!(state.workspace, Workspace::Browse);
    assert!(matches!(
        state.compare.session,
        Some(SessionPhase::Ready(1))
    ));
    assert!(state.compare.setup.source.picked().is_some());
}
