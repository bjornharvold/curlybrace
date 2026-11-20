//! Saved tabs that wait: for room under the limit, across a disconnect, for an unknown connection.
use super::*;

fn ab() -> (ConnectionConfig, ConnectionConfig) {
    (
        ConnectionConfig::new("a", "mongodb://a"),
        ConnectionConfig::new("b", "mongodb://b"),
    )
}

/// `a` with `open` tabs of its own, then `b` connected with `saved` tabs of which not all fit.
fn crowded(open: usize, saved: usize, active: Option<usize>) -> (AppState, Uuid, Uuid) {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let tabs: Vec<SavedTab> = (0..saved).map(|n| tab(idb, &format!("saved{n}"))).collect();
    let (s, _) = connected(waiting(vec![a, b], tabs, active), ida);
    let s = open_on(s, ida, open);
    let (s, _) = connected(s, idb);
    (s, ida, idb)
}

#[test]
fn tabs_that_did_not_fit_are_announced_once() {
    use crate::app::persistence::Recovery;
    let (s, _, idb) = crowded(10, 15, None);
    let left = s.restore.dormant[&idb].len();
    assert!(left > 0);
    assert_eq!(
        s.banner,
        Some(Banner::Recovery(Recovery::WorkspaceWaiting { count: left }))
    );
    assert!(s.pending_recovery.is_empty(), "once");
}

#[test]
fn closing_a_tab_reopens_a_waiting_one_dormant_and_without_reading() {
    let (s, ida, idb) = crowded(10, 15, None);
    let waiting_before = s.restore.dormant[&idb].len();
    let to_close: Vec<TabId> = s.tabs_of(ida).into_iter().take(3).collect();
    let mut s = s;
    for tab in to_close {
        let (next, commands) = update(s, Input::Tabs(TabAction::Close(tab)));
        s = next;
        assert!(
            !commands.iter().any(
                |c| matches!(c, Command::Find { ns, .. } if ns.collection.starts_with("saved"))
            ),
            "a reopened tab reads nothing: {commands:?}"
        );
    }
    let waiting_after = s.restore.dormant.get(&idb).map_or(0, Vec::len);
    assert!(
        waiting_after < waiting_before,
        "{waiting_before} -> {waiting_after}"
    );
    assert!(s.visible_tabs().len() <= MAX_TABS);
    for tab in s.tabs_of(idb) {
        if s.tabs.restored.contains(&tab) {
            assert_eq!(s.freshness_of(tab), Freshness::Dormant);
        }
    }
    // Oldest first: what is still waiting is the end of the saved list.
    let first_waiting = &s.restore.dormant[&idb][0].collection;
    let opened: Vec<String> = namespaces(&s, idb);
    assert!(
        opened
            .iter()
            .all(|c| c < first_waiting || c.len() < first_waiting.len()),
        "{opened:?} vs {first_waiting}"
    );
}

#[test]
fn the_saved_active_tab_keeps_its_place_when_it_did_not_fit() {
    let (s, _, idb) = crowded(17, 5, Some(4));
    let left = s.restore.dormant[&idb].len();
    let opened = namespaces(&s, idb).len();
    assert_eq!(opened + left, 5);
    assert!(left > 0, "some wait");
    assert_eq!(
        s.restore.active_hint,
        Some((idb, 4 - opened)),
        "the hint follows its tab"
    );
}

#[test]
fn waiting_tabs_stay_ahead_of_the_ones_left_open() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let (mut s, _) = connected(waiting(vec![cfg], vec![tab(id, "open")], Some(0)), id);
    s.restore.dormant.insert(id, vec![tab(id, "old")]);
    let (s, _) = update(s, Input::Disconnect(id));
    let order: Vec<&str> = s.restore.dormant[&id]
        .iter()
        .map(|t| t.collection.as_str())
        .collect();
    assert_eq!(order, ["old", "open"]);
}

#[test]
fn a_live_split_is_kept_waiting_after_a_disconnect() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let file = WorkspaceFile {
        tabs: vec![tab(id, "one"), tab(id, "two")],
        active: Some(0),
        split: Some(crate::app::workspace_file::SavedSplit {
            secondary: 1,
            ratio: 0.3,
            orientation: crate::app::workspace_file::SavedOrientation::Stacked,
            secondary_first: true,
        }),
        ..WorkspaceFile::default()
    };
    let mut s = AppState::new(vec![cfg]);
    s.restore = Box::new(RestoreState::of_file(&file, WorkspaceSync::default()));
    let (s, _) = connected(s, id);
    assert!(s.tabs.split.is_some(), "the split is back");
    let (s, _) = update(s, Input::Disconnect(id));
    assert_eq!(
        snapshot(&s).split,
        file.split,
        "and it survives the disconnect"
    );
}

#[test]
fn disconnecting_the_front_connection_remembers_its_front_tab_with_another_open() {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let s = waiting(vec![a, b], vec![tab(ida, "x"), tab(ida, "y")], Some(1));
    let (s, _) = connected(s, ida);
    let (s, _) = connected(s, idb);
    let (s, _) = update(s, Input::Activate(ida));
    assert_eq!(s.selected_ns().map(|n| n.collection.as_str()), Some("y"));
    let (s, _) = update(s, Input::Disconnect(ida));
    assert_eq!(s.restore.active_hint, Some((ida, 1)));
}

#[test]
fn a_deleted_connection_leaves_unverified_waiting_tabs_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let unknown = Uuid::new_v4();
    let mut s = waiting(
        vec![a, b],
        vec![tab(ida, "x"), tab(unknown, "u"), tab(idb, "y")],
        None,
    );
    let secrets = MemorySecretStore::default();
    let path = Ok(dir.path().join("connections.json"));
    s = crate::app::connections::deleted(s, ida, &secrets, &path).0;
    assert!(!s.restore.dormant.contains_key(&ida));
    assert!(
        s.restore.dormant.contains_key(&unknown),
        "unknown is not deleted"
    );
    assert!(s.restore.dormant.contains_key(&idb));
}
