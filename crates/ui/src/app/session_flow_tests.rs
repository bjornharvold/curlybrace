//! Several sessions open at once: opening, focusing, failing, closing and routing of events.
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{EditorAction, FindAction, SidebarAction};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

fn app(names: &[&str]) -> (AppState, Vec<Uuid>) {
    let cfgs: Vec<_> = names
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids = cfgs.iter().map(|c| c.id).collect();
    (AppState::new(cfgs), ids)
}

fn ev(state: AppState, event: Event) -> Out {
    update(state, Input::Event(event))
}

fn connected(session: Uuid, epoch: u64) -> Event {
    Event::Connected {
        session,
        epoch,
        topology: None,
    }
}

fn refused(session: Uuid, epoch: u64) -> Event {
    Event::ConnectFailed {
        session,
        epoch,
        error: CoreError::Database("refused".into()),
    }
}

/// Connects `id` and answers the Connect, so the session ends up active and `Connected`.
fn opened(state: AppState, id: Uuid) -> AppState {
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    ev(s, connected(id, epoch)).0
}

fn users() -> Namespace {
    Namespace::new("app", "users")
}

fn page_of(who: &str) -> Page {
    Page {
        docs: vec![doc! { "who": who }],
        skip: 0,
        has_more: false,
    }
}

fn results(session: Uuid, who: &str) -> Event {
    Event::Results {
        session,
        ns: users(),
        query: Query::default(),
        page: page_of(who),
    }
}

fn open_users(state: AppState) -> AppState {
    update(state, Input::Sidebar(SidebarAction::Open(users()))).0
}

fn who(page: &Option<Page>) -> Option<String> {
    page.as_ref()
        .and_then(|p| p.docs.first())
        .and_then(|d| d.get_str("who").ok().map(str::to_string))
}

#[test]
fn connecting_a_second_connection_parks_the_first_and_activates_the_second() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(s, a);
    let (s, _) = ev(
        s,
        Event::Databases {
            session: a,
            names: vec!["app".into()],
        },
    );
    let (s, cmds) = update(s, Input::Connect(b));
    assert_eq!(s.open, vec![a, b]);
    assert_eq!(s.active, Some(b));
    assert_eq!(s.status, ConnStatus::Connecting);
    assert_eq!(s.parked[&a].status, ConnStatus::Connected);
    assert_eq!(s.parked[&a].sidebar.databases.len(), 1);
    assert_eq!(s.epoch_counter, 2);
    assert_eq!(s.latest_requested_epoch, 2);
    assert!(matches!(cmds.as_slice(), [Command::Connect(cfg)] if cfg.id == b));
}

#[test]
fn connect_on_an_open_connection_focuses_it_and_sends_nothing() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    let (s, cmds) = update(s, Input::Connect(a));
    assert!(cmds.is_empty());
    assert_eq!(s.active, Some(a));
    assert_eq!(s.status, ConnStatus::Connected);
    assert_eq!(s.parked[&b].status, ConnStatus::Connected);
    assert_eq!(s.epoch_counter, 2, "no request was made");
}

#[test]
fn connect_on_a_connecting_root_also_only_focuses_it() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let (s, _) = update(s, Input::Connect(a));
    let s = opened(s, b);
    let (s, cmds) = update(s, Input::Connect(a));
    assert!(cmds.is_empty());
    assert_eq!((s.active, s.status), (Some(a), ConnStatus::Connecting));
}

#[test]
fn connect_on_a_failed_root_reconnects_it_with_a_new_epoch() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = ev(s, refused(a, 1));
    assert_eq!(s.status, ConnStatus::Failed);
    let (s, cmds) = update(s, Input::Connect(a));
    assert!(matches!(cmds.as_slice(), [Command::Connect(cfg)] if cfg.id == a));
    assert_eq!(s.open, vec![a]);
    assert_eq!(s.status, ConnStatus::Connecting);
    assert_eq!(s.failure, None);
    assert_eq!(s.banner, None);
    assert_eq!((s.epoch_counter, s.latest_requested_epoch), (2, 2));
}

#[test]
fn a_failed_background_root_reconnects_in_place() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = ev(s, refused(a, 1));
    let s = opened(s, b);
    let (s, cmds) = update(s, Input::Connect(a));
    assert!(matches!(cmds.as_slice(), [Command::Connect(cfg)] if cfg.id == a));
    assert_eq!(s.open, vec![a, b], "the root keeps its place");
    assert_eq!((s.active, s.status), (Some(a), ConnStatus::Connecting));
    assert_eq!(s.parked[&b].status, ConnStatus::Connected);
}

#[test]
fn closing_a_failed_root_still_sends_a_disconnect_that_removes_the_runtime_entry() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = ev(s, refused(a, 1));
    let (s, cmds) = update(s, Input::Disconnect(a));
    assert!(matches!(cmds.as_slice(), [Command::Disconnect { session }] if *session == a));
    assert!(s.open.is_empty());
}

#[test]
fn a_parked_session_receives_its_own_events() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    let (s, cmds) = ev(
        s,
        Event::Databases {
            session: a,
            names: vec!["x".into(), "y".into()],
        },
    );
    assert!(cmds.is_empty());
    assert_eq!(s.active, Some(b));
    assert!(s.sidebar.databases.is_empty(), "b is untouched");
    assert_eq!(s.parked[&a].sidebar.databases.len(), 2);
}

#[test]
fn a_connected_background_session_asks_for_its_own_databases() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = update(s, Input::Connect(b));
    let (s, cmds) = ev(s, connected(a, 1));
    assert!(matches!(cmds.as_slice(), [Command::ListDatabases { session }] if *session == a));
    assert_eq!(s.active, Some(b));
    assert_eq!(s.status, ConnStatus::Connecting);
    assert_eq!(s.parked[&a].status, ConnStatus::Connected);
}

#[test]
fn the_same_namespace_on_two_sessions_never_crosses() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = open_users(opened(opened(s, a), b));
    let (s, _) = update(
        s,
        Input::Sidebar(SidebarAction::InSession {
            session: a,
            action: Box::new(SidebarAction::Open(users())),
        }),
    );
    assert_eq!(s.active, Some(a));
    let (s, _) = ev(s, results(b, "b"));
    assert_eq!(who(&s.page), None, "a's page is not b's answer");
    assert_eq!(who(&s.parked[&b].page), Some("b".into()));
    let (s, _) = ev(s, results(a, "a"));
    assert_eq!(who(&s.page), Some("a".into()));
    assert_eq!(
        who(&s.parked[&b].page),
        Some("b".into()),
        "a's answer left b's page alone"
    );
}

#[test]
fn a_late_answer_of_a_closed_session_is_dropped_and_never_resurrects_the_root() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = opened(s, a);
    let (mut s, _) = update(s, Input::Disconnect(a));
    for late in [
        connected(a, 1),
        Event::Databases {
            session: a,
            names: vec!["x".into()],
        },
        results(a, "a"),
        refused(a, 1),
    ] {
        let (next, cmds) = ev(s, late);
        assert!(cmds.is_empty());
        assert!(next.open.is_empty() && next.active.is_none() && next.parked.is_empty());
        assert!(next.banner.is_none());
        s = next;
    }
}

#[test]
fn a_stale_connected_after_a_quick_reconnect_is_ignored() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = update(s, Input::Disconnect(a));
    let (s, _) = update(s, Input::Connect(a));
    assert_eq!(s.epoch_counter, 3);
    let (s, cmds) = ev(s, connected(a, 1));
    assert!(cmds.is_empty());
    assert_eq!(s.status, ConnStatus::Connecting);
    let (s, cmds) = ev(s, connected(a, 3));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(matches!(cmds.as_slice(), [Command::ListDatabases { session }] if *session == a));
}

#[test]
fn a_stale_failure_after_a_quick_reconnect_is_ignored() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = update(s, Input::Disconnect(a));
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = ev(s, refused(a, 1));
    assert_eq!(s.status, ConnStatus::Connecting);
    assert!(s.banner.is_none() && s.failure.is_none());
}

#[test]
fn disconnecting_the_active_session_activates_the_neighbour() {
    let (s, ids) = app(&["a", "b", "c"]);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let all = opened(opened(opened(s, a), b), c);

    let (s, cmds) = update(update(all, Input::Activate(b)).0, Input::Disconnect(b));
    assert_eq!(s.open, vec![a, c]);
    assert_eq!(s.active, Some(c), "the next root");
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(matches!(cmds.as_slice(), [Command::Disconnect { session }] if *session == b));
    assert!(!s.parked.contains_key(&b));

    let (s, _) = update(s, Input::Disconnect(c));
    assert_eq!(s.active, Some(a), "no next root: the previous one");
    assert_eq!(s.open, vec![a]);
}

#[test]
fn disconnecting_a_background_session_keeps_the_active_one() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    let (s, cmds) = update(s, Input::Disconnect(a));
    assert_eq!((s.active, s.open.clone()), (Some(b), vec![b]));
    assert!(s.parked.is_empty());
    assert_eq!(cmds.len(), 1);
    assert_eq!(s.epoch_counter, 3, "a disconnect takes an epoch");
}

#[test]
fn disconnecting_a_session_that_is_not_open_does_nothing() {
    let (s, _) = app(&["a"]);
    let (s, cmds) = update(s, Input::Disconnect(Uuid::new_v4()));
    assert!(cmds.is_empty() && s.epoch_counter == 0);
}

#[test]
fn disconnecting_the_last_session_leaves_nothing_active() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = open_users(opened(s, a));
    let (s, cmds) = update(s, Input::Disconnect(a));
    assert!(s.open.is_empty() && s.parked.is_empty());
    assert_eq!(s.active, None);
    assert_eq!(s.status, ConnStatus::Disconnected);
    assert!(s.page.is_none() && s.sidebar.selected.is_none() && s.pending.is_none());
    assert!(matches!(cmds.as_slice(), [Command::Disconnect { session }] if *session == a));
    assert_eq!(s.epoch_counter, 2);
}

#[test]
fn a_connect_failure_marks_only_that_root_failed() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = open_users(opened(s, a));
    let (s, _) = ev(s, results(a, "a"));
    let (s, _) = update(s, Input::Connect(b));
    let (s, cmds) = ev(s, refused(b, 2));
    assert!(cmds.is_empty());
    assert_eq!((s.active, s.status), (Some(b), ConnStatus::Failed));
    assert_eq!(s.failure, Some(CoreError::Database("refused".into())));
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "connect",
            ..
        })
    ));
    assert_eq!(s.open, vec![a, b], "the failed root stays in the tree");
    assert_eq!(s.parked[&a].status, ConnStatus::Connected);
    assert_eq!(who(&s.parked[&a].page), Some("a".into()));
    let roots = s.roots();
    assert_eq!(roots[0].status, ConnStatus::Connected);
    assert_eq!(roots[1].status, ConnStatus::Failed);
    assert!(roots[1].failure.is_some());
}

#[test]
fn a_connect_failure_of_a_background_session_does_not_touch_the_active_one() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let (s, _) = update(s, Input::Connect(a));
    let s = opened(s, b);
    let (s, _) = ev(s, refused(a, 1));
    assert_eq!((s.active, s.status), (Some(b), ConnStatus::Connected));
    assert_eq!(s.parked[&a].status, ConnStatus::Failed);
    assert!(s.failure.is_none());
}

#[test]
fn a_failed_list_of_databases_does_not_fail_the_root() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = opened(s, a);
    let (s, _) = ev(
        s,
        Event::Failed {
            session: a,
            context: "list_databases",
            error: CoreError::NotFound,
        },
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.failure.is_none() && s.banner.is_some());
}

#[test]
fn a_pending_write_of_one_session_does_not_block_another() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = open_users(opened(s, a));
    let save = || {
        Input::Editor(EditorAction::Save {
            original_id: None,
            text: "{}".into(),
        })
    };
    let (s, cmds) = update(s, save());
    assert!(matches!(cmds.as_slice(), [Command::Insert { session, .. }] if *session == a));
    assert!(s.pending.is_some());
    let s = open_users(opened(s, b));
    assert!(s.pending.is_none(), "b has its own marker");
    assert!(
        s.parked[&a].pending.is_some(),
        "a keeps its write in flight"
    );
    let (s, cmds) = update(s, save());
    assert!(matches!(cmds.as_slice(), [Command::Insert { session, .. }] if *session == b));
    assert!(s.pending.is_some() && s.parked[&a].pending.is_some());
}

#[test]
fn a_data_action_without_a_connected_active_session_raises_the_banner_and_sends_nothing() {
    let inputs = || {
        [
            Input::Find(FindAction::Clear),
            Input::Sidebar(SidebarAction::Refresh),
            Input::Sidebar(SidebarAction::Open(users())),
        ]
    };
    // Each state is rebuilt per input because `AppState` is not `Clone`.
    let nothing_active = |_: Uuid, s: AppState| s;
    let connecting = |id: Uuid, s: AppState| update(s, Input::Connect(id)).0;
    let failed = |id: Uuid, s: AppState| ev(connecting(id, s), refused(id, 1)).0;
    let states: [&dyn Fn(Uuid, AppState) -> AppState; 3] = [&nothing_active, &connecting, &failed];
    for make in states {
        for input in inputs() {
            let (s, ids) = app(&["a"]);
            let (next, cmds) = update(make(ids[0], s), input);
            assert!(cmds.is_empty());
            assert_eq!(next.banner, Some(Banner::NotConnected));
        }
    }
}

#[test]
fn a_sidebar_action_in_a_parked_session_toggles_without_activating_and_open_activates() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    let (s, _) = ev(
        s,
        Event::Databases {
            session: a,
            names: vec!["app".into()],
        },
    );
    let in_a = |action| {
        Input::Sidebar(SidebarAction::InSession {
            session: a,
            action: Box::new(action),
        })
    };
    let (s, cmds) = update(s, in_a(SidebarAction::LoadCollections("app".into())));
    assert!(matches!(cmds.as_slice(), [Command::ListCollections { session, .. }] if *session == a));
    assert_eq!(s.active, Some(b), "expanding does not move the focus");
    assert!(s.parked[&a].loading.contains("app"));
    assert!(s.parked[&a].sidebar.databases[0].expanded);

    let (s, cmds) = update(s, in_a(SidebarAction::Toggle("app".into())));
    assert!(cmds.is_empty());
    assert_eq!(s.active, Some(b));
    assert!(!s.parked[&a].sidebar.databases[0].expanded);

    let (s, cmds) = update(s, in_a(SidebarAction::Open(users())));
    assert!(matches!(cmds.as_slice(), [Command::Find { session, .. }] if *session == a));
    assert_eq!(
        s.active,
        Some(a),
        "opening a collection focuses its session"
    );
    assert_eq!(s.sidebar.selected, Some(users()));
    assert!(s.parked[&b].sidebar.selected.is_none());
}

#[test]
fn a_sidebar_action_in_a_closed_session_is_ignored() {
    let (s, ids) = app(&["a"]);
    let s = opened(s, ids[0]);
    let (s, cmds) = update(
        s,
        Input::Sidebar(SidebarAction::InSession {
            session: Uuid::new_v4(),
            action: Box::new(SidebarAction::Open(users())),
        }),
    );
    assert!(cmds.is_empty() && s.sidebar.selected.is_none() && s.banner.is_none());
}

#[test]
fn activate_focuses_an_open_session_and_ignores_others() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    let (s, cmds) = update(s, Input::Activate(a));
    assert!(cmds.is_empty());
    assert_eq!(s.active, Some(a));
    let (s, _) = update(s, Input::Activate(Uuid::new_v4()));
    assert_eq!(s.active, Some(a));
}
