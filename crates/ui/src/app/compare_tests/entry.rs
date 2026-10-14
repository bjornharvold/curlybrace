//! Entry points: reloading a failed listing, "Compare to..." from the sidebar, hiding.
use super::pairs::multi_state;
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
    assert_eq!(state.compare.preflight_view(), PreflightView::Ok);
}

fn connected_state() -> (AppState, uuid::Uuid) {
    let (state, a, _) = two_connections();
    let state = AppState {
        status: ConnStatus::Connected,
        active: Some(a),
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
fn the_default_source_follows_the_active_session() {
    let (state, a, b) = two_connections();
    let state = AppState {
        status: ConnStatus::Connected,
        open: vec![a, b],
        active: Some(a),
        ..state
    };
    let (state, _) = compare_to(state);
    assert_eq!(state.compare.connections, Some((a, a)));
    // Focus the other session and ask again: the default moves with it.
    let state = AppState {
        active: Some(b),
        compare: Default::default(),
        ..state
    };
    let (state, _) = compare_to(state);
    assert_eq!(state.compare.connections, Some((b, b)));
}

fn compare_database_to(state: AppState) -> (AppState, Vec<Command>) {
    update(
        state,
        Input::Sidebar(SidebarAction::CompareDatabaseTo("shop".into())),
    )
}

#[test]
fn opens_a_session_and_picks_the_source_database() {
    let (state, a) = connected_state();
    let (state, cmds) = compare_database_to(state);
    assert_eq!(state.workspace, Workspace::Compare);
    assert_eq!(state.compare.connections, Some((a, a)));
    let source = &state.compare.setup.source;
    assert_eq!(source.database.as_deref(), Some("shop"));
    assert!(source.collection.is_none());
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Open { session: 1, .. })]
    ));
}

#[test]
fn picks_the_same_database_on_the_target_when_the_connections_differ() {
    let (state, a, b) = two_connections();
    let state = AppState {
        status: ConnStatus::Connected,
        active: Some(a),
        ..state
    };
    let (state, _) = act(state, open_action(a, b));
    let (state, _) = update(state, ev(CompareEvent::Opened { session: 1 }));
    let (state, cmds) = compare_database_to(state);
    assert_eq!(state.compare.setup.source.database.as_deref(), Some("shop"));
    assert_eq!(state.compare.setup.target.database.as_deref(), Some("shop"));
    for side in [Side::Source, Side::Target] {
        assert!(cmds.iter().any(|c| matches!(
            c,
            Command::Compare(CompareCommand::Collections { side: s, db, .. })
                if *s == side && db == "shop"
        )));
    }
}

#[test]
fn leaves_the_target_database_unpicked_on_one_connection() {
    let (state, _) = connected_state();
    let (state, _) = compare_database_to(state);
    assert!(state.compare.setup.target.database.is_none());
}

#[test]
fn not_connected_shows_the_banner() {
    let (state, cmds) = compare_database_to(AppState::default());
    assert!(cmds.is_empty());
    assert_eq!(state.workspace, Workspace::Browse);
    assert_eq!(state.banner, Some(Banner::NotConnected));
}

#[test]
fn an_open_session_for_the_same_connections_is_reused_and_keeps_the_pair_list() {
    let state = multi_state(&["users", "orders"]);
    let (a, _) = state.compare.connections.expect("session connections");
    let state = AppState {
        status: ConnStatus::Connected,
        active: Some(a),
        ..state
    };
    let kept = state.compare.pairs.clone();
    assert_eq!(kept.entries.len(), 2);
    let (state, cmds) = compare_database_to(state);
    assert_eq!(state.compare.session, Some(SessionPhase::Ready(1)));
    assert_eq!(state.compare.pairs, kept);
    assert!(
        cmds.iter()
            .all(|c| !matches!(c, Command::Compare(CompareCommand::Open { .. })))
    );
}

#[test]
fn compare_to_without_a_connection_only_reports_it() {
    let (state, cmds) = compare_to(AppState::default());
    assert!(cmds.is_empty());
    assert_eq!(state.workspace, Workspace::Browse);
    assert_eq!(state.banner, Some(Banner::NotConnected));
}

#[test]
fn the_accepted_check_keeps_its_counts_until_the_form_is_edited() {
    let state = checked_state();
    let kept = state.compare.last_check().cloned().expect("counts kept");
    assert_eq!((kept.source_count, kept.target_count), (10, 10));
    let (state, _) = act(state, CompareAction::SetKey("sku".into()));
    assert!(state.compare.last_check().is_none());
}

#[test]
fn counts_are_cleared_by_a_recheck_a_stale_answer_and_a_failed_check() {
    let (rechecking, _) = act(checked_state(), CompareAction::RunCheck);
    assert!(rechecking.compare.last_check().is_none(), "while checking");

    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let (state, _) = act(state, CompareAction::SetKey("sku".into()));
    let (state, _) = update(state, checked_event(indexed(3)));
    assert!(state.compare.last_check().is_none(), "stale answer dropped");

    let (state, _) = act(picked_state(), CompareAction::RunCheck);
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_check",
        error: CoreError::NotFound,
    });
    let (state, _) = update(state, failed);
    assert!(state.compare.last_check().is_none());
}

#[test]
fn connect_records_the_connection_and_a_cleared_session_forgets_it() {
    let (state, a, _) = two_connections();
    let (state, _) = update(state, Input::Connect(a));
    assert_eq!(state.active, Some(a));
    let (closed, _) = update(update(state, Input::Disconnect(a)).0, Input::Connect(a));
    assert_eq!(closed.session_cleared().active, None);
}

#[test]
fn opening_a_collection_from_the_sidebar_leaves_the_compare_workspace() {
    let (state, a) = connected_state();
    let (state, _) = compare_to(state);
    assert_eq!(state.workspace, Workspace::Compare);
    let ns = Namespace::new("shop", "orders");
    let (state, _) = update(state, Input::Sidebar(SidebarAction::Open(ns.clone())));
    assert_eq!(state.workspace, Workspace::Browse);
    assert_eq!(state.selected_ns(), Some(&ns));
    assert_eq!(state.compare.connections, Some((a, a)), "session kept");
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

#[test]
fn a_cleared_session_returns_to_browse_but_keeps_the_compare_session() {
    let (state, a) = connected_state();
    let (state, _) = compare_to(state);
    assert_eq!(state.workspace, Workspace::Compare);
    let connections = state.compare.connections;
    let state = state.session_cleared();
    assert_eq!(state.workspace, Workspace::Browse);
    assert_eq!(state.compare.connections, connections);
    assert_eq!(connections, Some((a, a)));
}

#[test]
fn compare_to_falls_back_to_the_source_when_the_old_target_was_deleted() {
    let (mut state, a, b) = two_connections();
    state.conn.connections.retain(|c| c.id != b);
    let state = AppState {
        status: ConnStatus::Connected,
        active: Some(a),
        compare: CompareState {
            connections: Some((a, b)),
            ..state.compare.clone()
        },
        ..state
    };
    let (state, cmds) = compare_to(state);
    assert_eq!(state.compare.connections, Some((a, a)));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Open { .. }), ..]
    ));
    assert_eq!(state.workspace, Workspace::Compare);
}
