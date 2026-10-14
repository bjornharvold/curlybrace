//! Parking and restoring the views of several open sessions.
use super::*;
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::store::Namespace;

fn page_with(n: i32) -> Page {
    Page {
        docs: vec![doc! { "n": n }],
        skip: 0,
        has_more: false,
    }
}

fn editor(text: &str) -> EditorState {
    EditorState {
        original_id: None,
        text: text.into(),
        initial_text: text.into(),
        error: None,
    }
}

/// A fully populated view, so a lost field shows up as a difference.
fn busy_view(n: i32) -> SessionView {
    SessionView {
        status: ConnStatus::Connected,
        failure: Some(CoreError::SshHostKeyChanged),
        latest_requested_epoch: 7 + n as u64,
        topology: Some(Topology::ReplicaSet),
        sidebar: SidebarState::default(),
        loading: BTreeSet::from([format!("db{n}")]),
        find: FindBarState::default(),
        issued_query: Query {
            skip: 3 + n as u64,
            ..Query::default()
        },
        page_query: Query {
            skip: 5 + n as u64,
            ..Query::default()
        },
        loading_page: true,
        page: Some(page_with(n)),
        page_key: Some(PageKey {
            len: 1,
            skip: 0,
            fingerprint: n as u64,
        }),
        tree: TreeState::default(),
        view_mode: ViewMode::default(),
        editor: Some(editor(&format!("doc {n}"))),
        pending: Some(Pending {
            ns: Namespace::new("db", "c"),
            editor: None,
        }),
        ssh_endpoint: Some(HostEndpoint {
            host: format!("host{n}"),
            port: 22,
        }),
    }
}

fn open_two() -> (AppState, Uuid, Uuid) {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let mut s = AppState::new(vec![]);
    s.open = vec![a, b];
    s.active = Some(a);
    s.restore_view(busy_view(1));
    s.parked.insert(b, busy_view(2));
    (s, a, b)
}

/// The foreground fields and every parked view, rendered for comparison.
fn snapshot(s: &AppState) -> (String, Vec<(Uuid, String)>) {
    let view = SessionView {
        status: s.status,
        failure: s.failure.clone(),
        latest_requested_epoch: s.latest_requested_epoch,
        topology: s.topology,
        sidebar: s.sidebar.clone(),
        loading: s.loading.clone(),
        find: s.find.clone(),
        issued_query: s.issued_query.clone(),
        page_query: s.page_query.clone(),
        loading_page: s.loading_page,
        page: s.page.clone(),
        page_key: s.page_key,
        tree: s.tree.clone(),
        view_mode: s.view_mode,
        editor: s.editor.clone(),
        pending: s.pending.clone(),
        ssh_endpoint: s.ssh_endpoint.clone(),
    };
    let parked = s
        .parked
        .iter()
        .map(|(id, v)| (*id, format!("{v:?}")))
        .collect();
    (format!("{view:?}"), parked)
}

#[test]
fn activating_a_session_parks_the_current_view_and_restores_the_target() {
    let (s, a, b) = open_two();
    let next = s.activated(b);
    assert_eq!(next.active, Some(b));
    assert_eq!(next.page, Some(page_with(2)));
    assert_eq!(next.editor, Some(editor("doc 2")));
    assert!(!next.parked.contains_key(&b));
    assert_eq!(next.parked[&a].page, Some(page_with(1)));
    assert_eq!(next.parked[&a].editor, Some(editor("doc 1")));
}

#[test]
fn activating_twice_round_trips_every_field() {
    let (s, a, b) = open_two();
    let before = snapshot(&s);
    assert_eq!(format!("{:?}", busy_view(1)), before.0);
    let back = s.activated(b).activated(a);
    assert_eq!(back.active, Some(a));
    assert_eq!(snapshot(&back), before);
}

#[test]
fn activating_an_unknown_or_already_active_session_changes_nothing() {
    let (s, a, _) = open_two();
    let before = snapshot(&s);
    let same = s.activated(a);
    assert_eq!(snapshot(&same), before);
    let unknown = same.activated(Uuid::from_u128(99));
    assert_eq!(unknown.active, Some(a));
    assert_eq!(snapshot(&unknown), before);
}

#[test]
fn with_session_applies_to_the_parked_view_and_restores_the_active_one() {
    let (s, a, b) = open_two();
    let pending = Pending {
        ns: Namespace::new("other", "x"),
        editor: None,
    };
    let expected = pending.clone();
    let (next, commands) = s.with_session(b, |s| {
        assert_eq!(s.active, Some(b));
        (
            AppState {
                pending: Some(pending),
                ..s
            },
            vec![],
        )
    });
    assert!(commands.is_empty());
    assert_eq!(next.active, Some(a));
    assert_eq!(next.parked[&b].pending, Some(expected));
    assert_eq!(next.pending, busy_view(1).pending);
    assert_eq!(next.page, Some(page_with(1)));
}

#[test]
fn with_session_on_an_unknown_session_runs_nothing() {
    let (s, a, _) = open_two();
    let (next, commands) = s.with_session(Uuid::from_u128(99), |_| panic!("must not run"));
    assert!(commands.is_empty());
    assert_eq!(next.active, Some(a));
}

#[test]
fn with_session_drops_nothing_when_the_callback_closes_the_active_session() {
    let (s, a, b) = open_two();
    // Closing the session being worked on: the previous one is shown again.
    let (next, _) = s.with_session(b, |s| (s.session_dropped(b), vec![]));
    assert_eq!(next.open, vec![a]);
    assert_eq!(next.active, Some(a));
    assert_eq!(next.page, Some(page_with(1)));

    // Closing the previous one while b is shown: the state stays on b.
    let (s, a, b) = open_two();
    let (next, _) = s.with_session(b, |s| (s.session_dropped(a), vec![]));
    assert_eq!(next.active, Some(b));
    assert_eq!(next.open, vec![b]);
    assert_eq!(next.page, Some(page_with(2)));
}

#[test]
fn roots_follow_opening_order_and_flag_the_active_one() {
    let (mut s, a, b) = open_two();
    let mut alpha = ConnectionConfig::new("Alpha", "mongodb://db:27017");
    alpha.id = a;
    s.conn.connections = vec![alpha];
    let roots = s.roots();
    assert_eq!(roots.iter().map(|r| r.id).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(roots[0].name, "Alpha");
    assert_eq!(
        roots[1].name, "00000000",
        "a deleted connection shows its id"
    );
    assert_eq!(
        roots.iter().map(|r| r.active).collect::<Vec<_>>(),
        vec![true, false]
    );
    assert_eq!(roots[0].topology, Some(Topology::ReplicaSet));
    assert_eq!(roots[1].failure, Some(CoreError::SshHostKeyChanged));
    assert_eq!(roots[1].status, ConnStatus::Connected);

    let swapped = s.activated(b).roots();
    assert_eq!(
        swapped.iter().map(|r| r.active).collect::<Vec<_>>(),
        vec![false, true]
    );
    assert_eq!(swapped[0].id, a);
}

#[test]
fn session_dropped_removes_the_view_and_the_root() {
    let (s, a, b) = open_two();
    let next = s.session_dropped(b);
    assert!(!next.is_open(b));
    assert!(next.parked.is_empty());
    assert_eq!(next.roots().len(), 1);
    assert_eq!(next.active, Some(a));
    assert_eq!(next.page, Some(page_with(1)));
}

#[test]
fn dropping_the_active_session_leaves_no_active_view() {
    let (s, a, b) = open_two();
    let next = s.session_dropped(a);
    assert_eq!(next.active, None);
    assert_eq!(next.open, vec![b]);
    assert_eq!(next.page, None);
    assert_eq!(next.parked[&b].page, Some(page_with(2)));
}

#[test]
fn session_cleared_forgets_open_sessions_failure_and_topology() {
    let (s, _, _) = open_two();
    let cleared = s.session_cleared();
    assert!(cleared.open.is_empty());
    assert!(cleared.parked.is_empty());
    assert_eq!(cleared.active, None);
    assert_eq!(cleared.failure, None);
    assert_eq!(cleared.topology, None);
}
