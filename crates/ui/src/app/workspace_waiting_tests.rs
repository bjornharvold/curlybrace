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
fn waiting_tabs_come_after_the_ones_left_open() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let (mut s, _) = connected(waiting(vec![cfg], vec![tab(id, "open")], Some(0)), id);
    s.restore.dormant.insert(id, vec![tab(id, "old")]);
    let (s, _) = update(s, Input::Disconnect(id));
    let order: Vec<&str> = s.restore.dormant[&id]
        .iter()
        .map(|t| t.collection.as_str())
        .collect();
    assert_eq!(order, ["open", "old"]);
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

fn limit_notice(s: &AppState) -> bool {
    s.tabs.notice == Some(crate::app::tab_strip::TabNotice::LimitReached)
}

fn close_tab(s: AppState, tab: TabId) -> AppState {
    update(s, Input::Tabs(TabAction::Close(tab))).0
}

#[test]
fn closing_one_tab_reopens_one_waiting_tab_but_a_new_tab_still_opens() {
    let (s, ida, idb) = crowded(19, 10, None);
    let before = s.restore.dormant[&idb].len();
    let tabs: Vec<TabId> = s.tabs_of(ida).into_iter().take(2).collect();
    let mut s = s;
    for tab in tabs {
        s = close_tab(s, tab);
    }
    let after = s.restore.dormant[&idb].len();
    assert_eq!(
        before - after,
        1,
        "one slot of the two was given to a saved tab"
    );
    assert!(s.visible_tabs().len() < MAX_TABS, "a slot stays free");
    let shown = s.visible_tabs().len();
    let (s, _) = update(s, Input::Tabs(TabAction::New));
    assert!(!limit_notice(&s), "New tab works");
    assert!(s.visible_tabs().len() > shown || s.active_tab().is_some());
}

#[test]
fn close_others_does_not_refill() {
    let (s, ida, idb) = crowded(19, 10, None);
    let before = s.restore.dormant[&idb].len();
    let keep = s.tabs_of(ida)[0];
    let (s, _) = update(s, Input::Tabs(TabAction::CloseOthers(keep)));
    assert_eq!(
        s.restore.dormant[&idb].len(),
        before,
        "nothing was reopened"
    );
    assert!(namespaces(&s, idb).is_empty());
}

#[test]
fn reopen_closed_works_right_after_a_close_with_waiting_tabs() {
    let (s, ida, _) = crowded(19, 10, None);
    let tab = s.tabs_of(ida)[1];
    let name = s.tab_ns(tab).unwrap().collection;
    let s = close_tab(s, tab);
    let (s, _) = update(s, Input::Tabs(TabAction::ReopenClosed));
    assert!(!limit_notice(&s), "Undo close is not refused");
    assert!(namespaces(&s, ida).contains(&name), "the tab is back");
}

/// `crowded(10, 15)` with the strip then emptied of `a`'s other tabs: plenty of free room, and
/// still tabs waiting.
fn roomy() -> (AppState, Uuid, Uuid) {
    let (s, ida, idb) = crowded(10, 15, None);
    let keep = s.tabs_of(ida)[0];
    let (s, _) = update(s, Input::Tabs(TabAction::CloseOthers(keep)));
    assert!(!s.restore.dormant[&idb].is_empty(), "some still wait");
    assert!(s.visible_tabs().len() < MAX_TABS - 5, "and there is room");
    (s, ida, idb)
}

#[test]
fn one_close_reopens_exactly_one_waiting_tab_when_there_is_plenty_of_room() {
    let (s, ida, idb) = roomy();
    let before = s.restore.dormant[&idb].len();
    let opened_before = namespaces(&s, idb).len();
    let extra = open_on(s, ida, 2);
    let tab = extra.tabs_of(ida)[1];
    let s = close_tab(extra, tab);
    assert_eq!(s.restore.dormant[&idb].len(), before - 1, "one, not all");
    assert_eq!(namespaces(&s, idb).len(), opened_before + 1);
}

#[test]
fn a_refill_leaves_the_front_alone() {
    let (s, ida, idb) = roomy();
    let before = s.restore.dormant[&idb].len();
    let s = open_on(s, ida, 2);
    let (active, front) = (s.active, s.active_tab());
    let tab = s
        .tabs_of(ida)
        .into_iter()
        .find(|t| Some(*t) != front)
        .unwrap();
    let s = close_tab(s, tab);
    assert_eq!(
        s.restore.dormant[&idb].len(),
        before - 1,
        "a tab did reopen"
    );
    assert_eq!((s.active, s.active_tab()), (active, front));
}

#[test]
fn a_refill_prefers_the_connection_of_the_closed_tab() {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let mut saved: Vec<SavedTab> = (0..3).map(|n| tab(ida, &format!("a{n}"))).collect();
    saved.extend((0..3).map(|n| tab(idb, &format!("b{n}"))));
    let (s, _) = connected(waiting(vec![a, b], saved, None), ida);
    let (s, _) = connected(s, idb);
    let mut s = s;
    // Both have tabs waiting for room.
    s.restore.dormant.insert(ida, vec![tab(ida, "wa")]);
    s.restore.dormant.insert(idb, vec![tab(idb, "wb")]);
    let tab = s.tabs_of(idb)[0];
    let s = close_tab(s, tab);
    assert!(
        s.restore.dormant.get(&idb).is_none_or(Vec::is_empty),
        "b's own came back"
    );
    assert_eq!(s.restore.dormant[&ida].len(), 1, "a's still waits");
}

#[test]
fn closing_a_tab_that_is_not_there_frees_no_slot() {
    let (s, _, idb) = roomy();
    let before = s.restore.dormant[&idb].len();
    let (s, _) = update(s, Input::Tabs(TabAction::Close(TabId(9999))));
    assert_eq!(s.restore.dormant[&idb].len(), before);
}

#[test]
fn a_hint_of_a_background_connection_follows_its_tab_past_the_open_ones() {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let saved: Vec<SavedTab> = (0..25).map(|n| tab(idb, &format!("c{n:02}"))).collect();
    let (s, _) = connected(waiting(vec![a, b], saved, Some(22)), ida);
    let (s, _) = update(s, Input::Connect(idb));
    let epoch = s.epoch_counter;
    let (s, _) = update(s, Input::Activate(ida));
    let (s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: idb,
            epoch,
            topology: None,
        }),
    );
    let waiting_left = s.restore.dormant[&idb].len();
    assert!(waiting_left > 0 && s.restore.active_hint.is_some());
    let (s, _) = update(s, Input::Disconnect(idb));
    let (_, at) = s.restore.active_hint.expect("the hint is kept");
    assert_eq!(
        s.restore.dormant[&idb][at].collection, "c22",
        "it still names its tab"
    );
    let file = snapshot(&s);
    assert_eq!(file.tabs[file.active.unwrap()].collection, "c22");
}

#[test]
fn a_runtime_stop_records_the_front_of_the_connection_that_was_in_front() {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let mut saved = vec![tab(ida, "x"), tab(ida, "y")];
    saved.extend((0..5).map(|n| tab(idb, &format!("b{n}"))));
    let (s, _) = connected(waiting(vec![a, b], saved, Some(1)), ida);
    let (s, _) = connected(s, idb);
    let mut s = s;
    s.restore.active_hint = Some((idb, 3));
    let (s, _) = update(s, Input::Activate(ida));
    assert_eq!(
        s.selected_ns().map(|n| n.collection.clone()),
        Some("y".to_string())
    );
    let s = s.session_cleared();
    assert_eq!(
        s.restore.active_hint,
        Some((ida, 1)),
        "the front connection's tab wins"
    );
}

#[test]
fn the_waiting_groups_are_cut_across_connections_and_never_the_open_tabs() {
    use crate::app::persistence::Recovery;
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let saved: Vec<SavedTab> = (0..10).map(|n| tab(id, &format!("open{n}"))).collect();
    let (mut s, _) = connected(waiting(vec![cfg], saved, Some(0)), id);
    let (x, y) = (Uuid::new_v4(), Uuid::new_v4());
    s.restore
        .dormant
        .insert(x, (0..120).map(|n| tab(x, &format!("x{n}"))).collect());
    s.restore
        .dormant
        .insert(y, (0..78).map(|n| tab(y, &format!("y{n}"))).collect());
    let (s, _) = update(s, Input::Disconnect(id));
    let total: usize = s.restore.dormant.values().map(Vec::len).sum();
    assert_eq!(total, MAX_SAVED_TABS);
    assert_eq!(s.restore.dormant[&id].len(), 10, "the working set is whole");
    assert_eq!(s.restore.active_hint, Some((id, 0)), "with its front tab");
    let told = |r: &Recovery| matches!(r, Recovery::WorkspaceTrimmed { count: 8 });
    let shown = matches!(&s.banner, Some(Banner::Recovery(r)) if told(r));
    assert!(
        shown || s.pending_recovery.iter().any(told),
        "{:?}",
        s.banner
    );
}

#[path = "workspace_fit_tests.rs"]
mod fit;
