//! Open and waiting tabs kept within the file's one bound, and the notices that say so.
use super::*;

/// Open tabs, waiting tabs and the file's one bound: the snapshot is whole.
fn assert_never_cut(s: &AppState) {
    let live = s
        .visible_tabs()
        .into_iter()
        .filter(|t| s.tab_ns(*t).is_some() && s.connection(s.tabs.owner[t]).is_some())
        .count();
    let waiting: usize = s.restore.dormant.values().map(Vec::len).sum();
    assert!(
        live + waiting <= MAX_SAVED_TABS,
        "{live} open + {waiting} waiting"
    );
    assert_eq!(
        snapshot(s).tabs.len(),
        live + waiting,
        "the snapshot cut something"
    );
}

#[test]
fn driving_the_app_from_a_full_file_never_leaves_more_to_save_than_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let cfgs: Vec<ConnectionConfig> = (0..5)
        .map(|n| ConnectionConfig::new(&format!("c{n}"), "mongodb://x"))
        .collect();
    let ids: Vec<Uuid> = cfgs.iter().map(|c| c.id).collect();
    // Four connections with 49 saved tabs each: 196 of the file's 200 wait.
    let saved: Vec<SavedTab> = ids[..4]
        .iter()
        .flat_map(|id| (0..49).map(move |n| tab(*id, &format!("s{n}"))))
        .collect();
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file_of_tabs(saved)).unwrap();
    let mut s = AppState::new(cfgs).with_workspace_path(Ok(path));
    let step = |s: &mut AppState| {
        s.workspace_step(std::time::Instant::now());
        assert_never_cut(s);
    };
    step(&mut s);
    // The fifth connection has nothing saved: what it opens takes room from the waiting tabs.
    let (next, _) = connected(s, ids[4]);
    s = next;
    for _ in 0..3 {
        s = open_on(s, ids[4], 6);
        step(&mut s);
    }
    let live = s.visible_tabs().len();
    let waiting: usize = s.restore.dormant.values().map(Vec::len).sum();
    assert!(
        live + waiting <= MAX_SAVED_TABS && waiting < 196,
        "{live} + {waiting}: the fit ran"
    );
    // Going back to waiting, coming back, and the runtime stopping all keep to the bound.
    let (next, _) = update(s, Input::Disconnect(ids[4]));
    s = next;
    step(&mut s);
    let (next, _) = connected(s, ids[1]);
    s = next;
    step(&mut s);
    s = s.session_cleared();
    step(&mut s);
}

#[test]
fn a_runtime_stop_raises_one_trimmed_notice_for_all_the_sessions() {
    use crate::app::persistence::Recovery;
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let (s, _) = connected(waiting(vec![a, b], vec![], None), ida);
    let (s, _) = connected(s, idb);
    let s = open_on(s, ida, 5);
    let mut s = open_on(s, idb, 5);
    let x = Uuid::new_v4();
    s.restore
        .dormant
        .insert(x, (0..195).map(|n| tab(x, &format!("x{n}"))).collect());
    let s = s.session_cleared();
    let trimmed = |r: &Recovery| match r {
        Recovery::WorkspaceTrimmed { count } => Some(*count),
        _ => None,
    };
    let mut all: Vec<usize> = s.pending_recovery.iter().filter_map(trimmed).collect();
    if let Some(Banner::Recovery(r)) = &s.banner {
        all.extend(trimmed(r));
    }
    assert_eq!(all.len(), 1, "one notice: {all:?}");
    assert_eq!(all, [5], "cut once, five tabs");
    assert_never_cut(&s);
}

#[test]
fn a_full_file_that_the_app_wrote_loads_back_whole_and_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let cfgs: Vec<ConnectionConfig> = (0..3)
        .map(|n| ConnectionConfig::new(&format!("c{n}"), "mongodb://x"))
        .collect();
    let ids: Vec<Uuid> = cfgs.iter().map(|c| c.id).collect();
    let mut saved: Vec<SavedTab> = (0..100).map(|n| tab(ids[1], &format!("b{n}"))).collect();
    saved.extend((0..80).map(|n| tab(ids[2], &format!("c{n}"))));
    let (s, _) = connected(waiting(cfgs.clone(), saved, None), ids[0]);
    let s = open_on(s, ids[0], MAX_TABS);
    let written = snapshot(&s);
    assert_eq!(
        written.tabs.len(),
        MAX_SAVED_TABS,
        "20 open and 180 waiting"
    );
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &written).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut next = AppState::new(cfgs).with_workspace_path(Ok(path.clone()));
    let waiting_now: usize = next.restore.dormant.values().map(Vec::len).sum();
    assert_eq!(waiting_now, MAX_SAVED_TABS, "nothing was lost");
    assert!(
        next.banner.is_none() && next.pending_recovery.is_empty(),
        "and nothing is reported"
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
    assert_eq!(leftovers.len(), 1, "no backup was made");
    next.workspace_step(std::time::Instant::now());
    assert_eq!(next.restore.sync.writes, 0);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "same bytes after a frame"
    );
}

#[test]
fn waiting_tabs_make_room_for_tabs_opened_after_a_full_load() {
    use crate::app::persistence::Recovery;
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let saved: Vec<SavedTab> = (0..MAX_SAVED_TABS)
        .map(|n| tab(idb, &format!("w{n}")))
        .collect();
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file_of_tabs(saved)).unwrap();
    let s = AppState::new(vec![a, b]).with_workspace_path(Ok(path));
    let (s, _) = connected(s, ida);
    let mut s = open_on(s, ida, 5);
    s.workspace_step(std::time::Instant::now());
    let total: usize = s.restore.dormant.values().map(Vec::len).sum();
    assert_eq!(total, MAX_SAVED_TABS - 5, "the oldest waiting made room");
    assert_never_cut(&s);
    let told = |r: &Recovery| matches!(r, Recovery::WorkspaceTrimmed { count: 5 });
    let shown = matches!(&s.banner, Some(Banner::Recovery(r)) if told(r));
    assert!(
        shown || s.pending_recovery.iter().any(told),
        "and it is said"
    );
}

fn file_of_tabs(tabs: Vec<SavedTab>) -> WorkspaceFile {
    WorkspaceFile {
        tabs,
        ..WorkspaceFile::default()
    }
}

fn waiting_notices(s: &AppState) -> usize {
    use crate::app::persistence::Recovery;
    let is = |r: &Recovery| matches!(r, Recovery::WorkspaceWaiting { .. });
    usize::from(matches!(&s.banner, Some(Banner::Recovery(r)) if is(r)))
        + s.pending_recovery.iter().filter(|r| is(r)).count()
}

#[test]
fn a_repeated_overflow_replaces_the_waiting_notice_instead_of_stacking() {
    let (s, _, idb) = crowded(10, 15, None);
    assert_eq!(waiting_notices(&s), 1);
    let (s, _) = update(s, Input::Disconnect(idb));
    let (s, _) = connected(s, idb);
    assert_eq!(waiting_notices(&s), 1, "one notice, with the new count");
}

#[test]
fn the_waiting_notice_goes_when_the_connection_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _, idb) = crowded(10, 15, None);
    let path = Ok(dir.path().join("connections.json"));
    let s = crate::app::connections::deleted(s, idb, &MemorySecretStore::default(), &path).0;
    assert_eq!(waiting_notices(&s), 0);
}

#[test]
fn the_waiting_notice_goes_when_restore_is_turned_off() {
    use crate::app::tools_state::ToolsAction;
    let (s, _, _) = crowded(10, 15, None);
    let (s, _) = update(s, Input::Tools(ToolsAction::SetRestore(false)));
    assert_eq!(waiting_notices(&s), 0);
}

#[test]
fn the_waiting_notice_goes_when_every_waiting_tab_has_reopened() {
    let (s, ida, idb) = crowded(10, 15, None);
    let mut s = s;
    for _ in 0..30 {
        if s.restore.dormant.get(&idb).is_none_or(Vec::is_empty) {
            break;
        }
        let tab = s.tabs_of(ida)[0];
        s = close_tab(s, tab);
    }
    assert!(
        s.restore.dormant.get(&idb).is_none_or(Vec::is_empty),
        "all reopened"
    );
    assert_eq!(waiting_notices(&s), 0);
}
#[test]
fn a_refill_skips_a_session_that_is_not_connected() {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let saved: Vec<SavedTab> = (0..5).map(|n| tab(idb, &format!("saved{n}"))).collect();
    let (s, _) = connected(waiting(vec![a, b], saved, None), ida);
    let s = open_on(s, ida, 5);
    // `b` is being connected: it is open, and not Connected.
    let (s, _) = update(s, Input::Connect(idb));
    let tabs: Vec<TabId> = s.tabs_of(ida).into_iter().take(2).collect();
    let mut s = s;
    for tab in tabs {
        s = close_tab(s, tab);
    }
    assert_eq!(s.restore.dormant[&idb].len(), 5, "still waiting");
    assert!(namespaces(&s, idb).is_empty());
}

#[test]
fn a_disconnect_and_reconnect_bring_the_working_set_back_first() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let saved: Vec<SavedTab> = (0..30).map(|n| tab(id, &format!("c{n:02}"))).collect();
    let (s, _) = connected(waiting(vec![cfg], saved, Some(0)), id);
    assert_eq!(namespaces(&s, id).len(), 20);
    assert_eq!(s.restore.dormant[&id].len(), 10);
    let (s, _) = update(s, Input::Disconnect(id));
    let group: Vec<&str> = s.restore.dormant[&id]
        .iter()
        .map(|t| t.collection.as_str())
        .collect();
    assert_eq!(group.len(), 30);
    assert_eq!(group[0], "c00", "the open tabs come first");
    assert_eq!(group[19], "c19");
    assert_eq!(group[20], "c20", "the older waiting ones after");
    let (s, _) = connected(s, id);
    let back = namespaces(&s, id);
    assert_eq!(back.first().map(String::as_str), Some("c00"));
    assert_eq!(back.last().map(String::as_str), Some("c19"), "{back:?}");
}

#[test]
fn a_full_file_drops_the_oldest_waiting_tabs_and_says_so() {
    use crate::app::persistence::Recovery;
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let saved: Vec<SavedTab> = (0..3).map(|n| tab(id, &format!("open{n}"))).collect();
    let (mut s, _) = connected(waiting(vec![cfg], saved, Some(0)), id);
    let old: Vec<SavedTab> = (0..MAX_SAVED_TABS)
        .map(|n| tab(id, &format!("old{n}")))
        .collect();
    s.restore.dormant.insert(id, old);
    let (s, _) = update(s, Input::Disconnect(id));
    let group = &s.restore.dormant[&id];
    assert_eq!(group.len(), MAX_SAVED_TABS);
    assert_eq!(group[0].collection, "open0", "the working set is kept");
    let told = |r: &Recovery| matches!(r, Recovery::WorkspaceTrimmed { count: 3 });
    let shown = matches!(&s.banner, Some(Banner::Recovery(r)) if told(r));
    assert!(
        shown || s.pending_recovery.iter().any(told),
        "the cut is announced"
    );
}

#[test]
fn the_waiting_notice_survives_another_connection_connecting() {
    use crate::app::persistence::Recovery;
    let (mut s, _, idb) = crowded(10, 15, None);
    let left = s.restore.dormant[&idb].len();
    let c = ConnectionConfig::new("c", "mongodb://c");
    let idc = c.id;
    s.conn.connections.push(c);
    let (s, _) = connected(s, idc);
    assert_eq!(
        s.banner,
        Some(Banner::Recovery(Recovery::WorkspaceWaiting { count: left }))
    );
}

#[test]
fn a_connection_that_is_not_in_front_leaves_another_ones_hints_alone() {
    let (a, c) = ab();
    let (ida, idc) = (a.id, c.id);
    let file = WorkspaceFile {
        tabs: vec![tab(idc, "x"), tab(idc, "y")],
        active: Some(0),
        split: Some(crate::app::workspace_file::SavedSplit {
            secondary: 1,
            ratio: 0.4,
            orientation: crate::app::workspace_file::SavedOrientation::Stacked,
            secondary_first: false,
        }),
        ..WorkspaceFile::default()
    };
    let mut s = AppState::new(vec![a, c]);
    s.restore = Box::new(RestoreState::of_file(&file, WorkspaceSync::default()));
    let (s, _) = connected(s, ida);
    let (s, _) = update(
        s,
        Input::Sidebar(crate::viewmodel::SidebarAction::Open(Namespace::new(
            "app", "mine",
        ))),
    );
    // `a` is not in front when it goes (another connection is): it takes no slot.
    let mut s = s;
    let d = ConnectionConfig::new("d", "mongodb://d");
    let idd = d.id;
    s.conn.connections.push(d);
    let (s, _) = connected(s, idd);
    let (s, _) = update(s, Input::Activate(idd));
    let (s, _) = update(s, Input::Disconnect(ida));
    assert!(
        s.restore.split_hint.is_some_and(|(id, _)| id == idc),
        "kept for c"
    );
    assert_eq!(s.restore.active_hint.map(|(id, _)| id), Some(idc));
}

#[test]
fn a_tab_whose_options_were_dropped_says_so_until_it_runs() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let flagged = SavedTab {
        options_dropped: true,
        ..tab(id, "users")
    };
    let (s, _) = connected(waiting(vec![cfg], vec![flagged], Some(0)), id);
    assert!(s.active_tab_options_dropped());
    assert!(
        snapshot(&s).tabs[0].options_dropped,
        "and the file keeps the flag"
    );
    let (mut s, _) = (s, ());
    s.explain.guard = false;
    let front = s.active_tab().unwrap();
    let (s, _) = update(s, Input::Tabs(TabAction::Run(front)));
    assert!(!s.active_tab_options_dropped(), "Run wakes the tab");
}

#[test]
fn the_waiting_notice_counts_what_waits_now_and_only_on_connected_sessions() {
    use crate::app::persistence::Recovery;
    let (s, ida, idb) = crowded(10, 15, None);
    let left = |s: &AppState| s.restore.dormant.get(&idb).map_or(0, Vec::len);
    let shown = |s: &AppState| match &s.banner {
        Some(Banner::Recovery(Recovery::WorkspaceWaiting { count })) => Some(*count),
        _ => None,
    };
    assert_eq!(shown(&s), Some(left(&s)));
    // A close reopens one: the count follows.
    let tab = s.tabs_of(ida)[0];
    let s = close_tab(s, tab);
    assert_eq!(shown(&s), Some(left(&s)), "updated as tabs reopen");
    // Disconnecting the connection withdraws it.
    let (s, _) = update(s, Input::Disconnect(idb));
    assert_eq!(shown(&s), None);
    // A connection that is being connected is not "waiting for room".
    let (s, _) = update(s, Input::Connect(idb));
    assert_eq!(s.waiting_for_room(), 0);
}

#[test]
fn the_front_connection_wins_the_files_one_hint_slot() {
    let (a, c) = ab();
    let (ida, idc) = (a.id, c.id);
    let file = WorkspaceFile {
        tabs: vec![tab(idc, "x"), tab(idc, "y")],
        active: Some(0),
        split: Some(crate::app::workspace_file::SavedSplit {
            secondary: 1,
            ratio: 0.4,
            orientation: crate::app::workspace_file::SavedOrientation::Stacked,
            secondary_first: false,
        }),
        ..WorkspaceFile::default()
    };
    let mut s = AppState::new(vec![a, c]);
    s.restore = Box::new(RestoreState::of_file(&file, WorkspaceSync::default()));
    let (s, _) = connected(s, ida);
    let (s, _) = update(
        s,
        Input::Sidebar(crate::viewmodel::SidebarAction::Open(Namespace::new(
            "app", "mine",
        ))),
    );
    let (s, _) = update(s, Input::Disconnect(ida));
    let written = snapshot(&s);
    let front = &written.tabs[written.active.expect("a front tab")];
    assert_eq!(
        front.collection, "mine",
        "the file's one hint is the front connection's"
    );
    assert_eq!(written.split, None, "and c's split is not written");
    // c connects in front: it comes back without its saved front tab.
    let (s, _) = connected(s, idc);
    assert_eq!(namespaces(&s, idc), ["x", "y"], "its tabs are there");
    assert_eq!(s.tab_ns(s.active_tab().unwrap()), None, "but not in front");
}

#[test]
fn the_waiting_notice_count_is_current_after_the_fit_trims() {
    use crate::app::persistence::Recovery;
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let saved: Vec<SavedTab> = (0..199).map(|n| tab(idb, &format!("w{n}"))).collect();
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file_of_tabs(saved)).unwrap();
    let s = AppState::new(vec![a, b]).with_workspace_path(Ok(path));
    let (s, _) = connected(s, idb);
    let (s, _) = connected(s, ida);
    let mut s = open_on(s, ida, 3);
    // More wait than the file can hold next to the tabs that are open.
    let extra: Vec<SavedTab> = (0..40).map(|n| tab(idb, &format!("x{n}"))).collect();
    s.restore.dormant.get_mut(&idb).unwrap().extend(extra);
    s.workspace_step(std::time::Instant::now());
    let left = s.restore.dormant[&idb].len();
    let counts = |r: &Recovery| match r {
        Recovery::WorkspaceWaiting { count } => Some(*count),
        _ => None,
    };
    let mut seen: Vec<usize> = s.pending_recovery.iter().filter_map(counts).collect();
    if let Some(Banner::Recovery(r)) = &s.banner {
        seen.extend(counts(r));
    }
    assert_eq!(seen, [left], "the notice says how many wait now");
}

#[test]
fn a_runtime_stop_withdraws_the_waiting_notice() {
    let (s, _, _) = crowded(10, 15, None);
    assert_eq!(waiting_notices(&s), 1);
    let s = s.session_cleared();
    assert_eq!(
        waiting_notices(&s),
        0,
        "nothing is connected, nothing waits for room"
    );
}

#[test]
fn a_copy_and_a_reopened_tab_keep_the_options_dropped_note_and_flag() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let flagged = SavedTab {
        options_dropped: true,
        ..tab(id, "users")
    };
    let (s, _) = connected(
        waiting(vec![cfg], vec![flagged, tab(id, "orders")], Some(1)),
        id,
    );
    let users = restored_tab(&s, "users");
    let (s, _) = update(s, Input::Tabs(TabAction::Duplicate(users)));
    let copy = s.active_tab().expect("the copy");
    assert_ne!(copy, users);
    assert!(
        s.tabs.options_dropped.contains(&copy),
        "the copy carries it"
    );
    let flag_of = |s: &AppState, tab: TabId| {
        let file = snapshot(s);
        let at = s.visible_tabs().iter().position(|t| *t == tab).unwrap();
        file.tabs[at].options_dropped
    };
    assert!(flag_of(&s, copy), "and the next write keeps it");
    let (s, _) = update(s, Input::Tabs(TabAction::Close(copy)));
    let (s, _) = update(s, Input::Tabs(TabAction::ReopenClosed));
    let back = s.active_tab().expect("the reopened tab");
    assert!(s.tabs.options_dropped.contains(&back), "a reopened tab too");
    assert!(flag_of(&s, back));
}

#[test]
fn closing_the_only_tab_of_the_app_does_not_reopen_a_waiting_one() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let saved: Vec<SavedTab> = (0..30).map(|n| tab(id, &format!("c{n:02}"))).collect();
    let (s, _) = connected(waiting(vec![cfg], saved, Some(0)), id);
    let keep = s.active_tab().expect("the front tab");
    let (s, _) = update(s, Input::Tabs(TabAction::CloseOthers(keep)));
    let before = s.restore.dormant[&id].len();
    assert!(before > 0 && s.visible_tabs().len() == 1);
    // The only tab is emptied and stays: the strip lost nothing, so no slot was freed.
    let s = close_tab(s, keep);
    assert_eq!(s.restore.dormant[&id].len(), before, "nothing reopened");
    assert_eq!(s.visible_tabs().len(), 1);
}

/// A state with a workspace path whose file is absent, so `workspace_step` really runs.
fn stepping(cfgs: Vec<ConnectionConfig>) -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    (AppState::new(cfgs).with_workspace_path(Ok(path)), dir)
}

#[test]
fn a_live_tab_whose_text_the_file_could_not_take_says_so_while_it_runs() {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let (s, _dir) = stepping(vec![a, b]);
    let (s, _) = connected(s, ida);
    let mut s = open_on(s, ida, 1);
    s.find.text.filter = "x".repeat(255 * 1024);
    // So much waits that the largest tab, this one, cannot keep its text.
    let waiting_tabs: Vec<SavedTab> = (0..40)
        .map(|n| SavedTab {
            find: Some(SavedFind {
                filter: "y".repeat(250 * 1024 - n),
                ..SavedFind::default()
            }),
            ..tab(idb, &format!("w{n}"))
        })
        .collect();
    s.restore.dormant.insert(idb, waiting_tabs);
    s.workspace_step(std::time::Instant::now());
    assert!(
        s.active_tab_text_dropped(),
        "the running tab carries the note"
    );
    let front = snapshot(&s);
    assert!(front.tabs[0].text_dropped && front.tabs[0].find.is_none());
}

#[test]
fn the_remembered_front_tab_goes_once_a_saved_tab_is_in_front() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let (s, _dir) = stepping(vec![cfg]);
    let file = WorkspaceFile {
        tabs: vec![tab(id, "one"), tab(id, "two")],
        active: Some(1),
        ..WorkspaceFile::default()
    };
    let mut s = s;
    s.restore = Box::new(RestoreState::of_file(
        &file,
        std::mem::take(&mut s.restore.sync),
    ));
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    let mut s = s;
    s.workspace = Workspace::Compare;
    let (s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    );
    assert!(s.restore.front_live.is_some());
    // A saved tab comes to the front: the remembered one is no longer needed.
    let one = restored_tab(&s, "one");
    let (mut s, _) = update(s, Input::Tabs(TabAction::Activate(one)));
    s.workspace_step(std::time::Instant::now());
    assert_eq!(s.restore.front_live, None);
    // A new, empty tab is not "two" in front again.
    let (mut s, _) = update(s, Input::Tabs(TabAction::New));
    s.workspace_step(std::time::Instant::now());
    let file = snapshot(&s);
    let front = file.active.map(|at| file.tabs[at].collection.clone());
    assert_ne!(front.as_deref(), Some("two"), "{front:?}");
}

/// A running tab with 255 KiB of text in a file that already holds forty large waiting tabs, so
/// the fit drops this tab's text; and the state, stepped once.
fn dropped_by_the_fit() -> (AppState, Uuid, tempfile::TempDir) {
    let (a, b) = ab();
    let (ida, idb) = (a.id, b.id);
    let (s, dir) = stepping(vec![a, b]);
    let (s, _) = connected(s, ida);
    let mut s = open_on(s, ida, 1);
    s.find.text.filter = "x".repeat(255 * 1024);
    s.restore.dormant.insert(idb, big_waiting(idb));
    s.workspace_step(std::time::Instant::now());
    assert!(
        s.active_tab_text_dropped(),
        "the fit dropped this tab's text"
    );
    (s, idb, dir)
}

fn big_waiting(id: Uuid) -> Vec<SavedTab> {
    (0..40)
        .map(|n| SavedTab {
            find: Some(SavedFind {
                filter: "y".repeat(250 * 1024 - n),
                ..SavedFind::default()
            }),
            ..tab(id, &format!("w{n}"))
        })
        .collect()
}

#[test]
fn the_note_goes_when_the_text_comes_to_fit_again() {
    let (mut s, _, _dir) = dropped_by_the_fit();
    s.restore.dormant.clear();
    s.workspace_step(std::time::Instant::now());
    s.workspace_step(std::time::Instant::now());
    assert!(!s.active_tab_text_dropped(), "nothing is dropped now");
    let file = snapshot(&s);
    assert!(!file.tabs[0].text_dropped && file.tabs[0].find.is_some());
}

#[test]
fn a_tab_the_fit_dropped_and_the_person_emptied_loses_the_note_and_the_flag() {
    let (mut s, idb, _dir) = dropped_by_the_fit();
    s.find.text.filter = String::new();
    s.workspace_step(std::time::Instant::now());
    s.workspace_step(std::time::Instant::now());
    assert!(
        !s.active_tab_text_dropped(),
        "an empty tab has nothing dropped"
    );
    assert!(
        !snapshot(&s).tabs[0].text_dropped,
        "and the file does not say so"
    );
    // The same again later: the flag was not stuck.
    s.find.text.filter = "x".repeat(255 * 1024);
    s.restore.dormant.insert(idb, big_waiting(idb));
    s.workspace_step(std::time::Instant::now());
    s.workspace_step(std::time::Instant::now());
    assert!(s.active_tab_text_dropped());
}

#[test]
fn a_restored_empty_tab_keeps_its_flag_until_it_runs() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let dropped = SavedTab {
        find: None,
        text_dropped: true,
        ..tab(id, "users")
    };
    let (s, _) = connected(waiting(vec![cfg], vec![dropped], Some(0)), id);
    assert!(
        snapshot(&s).tabs[0].text_dropped,
        "restored and empty: still flagged"
    );
}

#[test]
fn turning_restore_off_clears_the_marks_of_running_tabs() {
    use crate::app::tools_state::ToolsAction;
    let (s, _, _dir) = dropped_by_the_fit();
    let (s, _) = update(s, Input::Tools(ToolsAction::SetRestore(false)));
    assert!(!s.active_tab_text_dropped());
}

#[test]
fn marks_go_while_saving_is_off() {
    let (mut s, _, _dir) = dropped_by_the_fit();
    s.restore.sync.guard = PersistGuard::blocked();
    s.workspace_step(std::time::Instant::now());
    assert!(!s.active_tab_text_dropped());
}
