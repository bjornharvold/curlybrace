//! Reopening the saved tabs: nothing is sent before a connect, a connect sends only the database
//! list, and the reopened tabs are dormant (their Run is the only thing that reads).
use super::*;
use crate::app::AggregateAction;
use crate::app::persistence::PersistGuard;
use crate::app::pipelines_wiring_tests::app_in;
use crate::app::reducer::{Input, update};
use crate::app::state::{Banner, ConnStatus};
use crate::app::tab_actions::TabAction;
use crate::app::tabs::Freshness;
use crate::app::tabs::test_support::parked;
use crate::app::workspace_file::{SavedSubTab, SavedViewMode, save};
use crate::viewmodel::{CollectionTab, EditorAction, tab_title::labels};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::saved_pipeline::SavedStage;
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::store::Namespace;
use eframe::egui;

fn tab(connection: Uuid, collection: &str) -> SavedTab {
    SavedTab {
        connection,
        db: "app".into(),
        collection: collection.into(),
        sub_tab: SavedSubTab::Documents,
        view_mode: SavedViewMode::Table,
        find: Some(SavedFind {
            filter: format!(r#"{{"from": "{collection}"}}"#),
            ..SavedFind::default()
        }),
        draft: None,
        text_dropped: false,
    }
}

fn stage(text: &str) -> SavedStage {
    SavedStage {
        text: text.into(),
        enabled: true,
    }
}

fn waiting(configs: Vec<ConnectionConfig>, tabs: Vec<SavedTab>, active: Option<usize>) -> AppState {
    let file = WorkspaceFile {
        tabs,
        active,
        ..WorkspaceFile::default()
    };
    let mut s = AppState::new(configs);
    s.restore = Box::new(RestoreState::of_file(&file, WorkspaceSync::default()));
    s
}

/// Connect asked for and answered: the state and what the answer sent.
fn connected(s: AppState, id: Uuid) -> (AppState, Vec<Command>) {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    )
}

fn only_list_databases(id: Uuid, commands: &[Command]) {
    assert!(
        matches!(commands, [Command::ListDatabases { session }] if *session == id),
        "{commands:?}"
    );
}

fn namespaces(s: &AppState, id: Uuid) -> Vec<String> {
    s.tabs_of(id)
        .into_iter()
        .filter_map(|t| s.tab_ns(t))
        .map(|ns| ns.collection)
        .collect()
}

fn finds(commands: &[Command]) -> Vec<&Command> {
    commands
        .iter()
        .filter(|c| matches!(c, Command::Find { .. }))
        .collect()
}

fn restored_tab(s: &AppState, collection: &str) -> TabId {
    s.tabs
        .order
        .iter()
        .copied()
        .find(|t| s.tab_ns(*t).is_some_and(|ns| ns.collection == collection))
        .expect("a restored tab")
}

#[test]
fn nothing_is_sent_between_launch_and_connect() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let file = WorkspaceFile {
        tabs: vec![tab(cfg.id, "users"), tab(cfg.id, "orders")],
        active: Some(1),
        ..WorkspaceFile::default()
    };
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file).unwrap();
    let (app, _events, mut commands) = app_in(&dir, vec![cfg.clone()]);
    let mut app = app.with_workspace_path(Ok(path));
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
    }
    assert!(commands.try_recv().is_err(), "nothing was sent at launch");
    assert!(app.state().open.is_empty(), "no connection was opened");
    assert!(app.state().tabs.order.is_empty(), "the strip is empty");
    assert_eq!(app.state().restore.dormant[&cfg.id].len(), 2);
}

#[test]
fn connect_materialises_tabs_and_sends_only_list_databases() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let s = waiting(
        vec![cfg],
        vec![tab(id, "users"), tab(id, "orders")],
        Some(1),
    );
    let (s, commands) = connected(s, id);
    only_list_databases(id, &commands);
    assert_eq!(namespaces(&s, id), ["users", "orders"]);
    assert_eq!(s.visible_tabs().len(), 2, "the empty first tab is gone");
    for tab in s.tabs_of(id) {
        assert_eq!(s.freshness_of(tab), Freshness::Dormant);
    }
    assert_eq!(
        s.active_tab(),
        Some(restored_tab(&s, "orders")),
        "the saved active tab"
    );
    assert_eq!(s.find.text.filter, r#"{"from": "orders"}"#);
    assert_eq!(s.tab, CollectionTab::Documents);
    let users = restored_tab(&s, "users");
    assert_eq!(parked(&s, users).find.text.filter, r#"{"from": "users"}"#);
    assert!(s.restore.dormant.is_empty(), "the saved tabs became tabs");
}

#[test]
fn a_restored_documents_tab_sends_no_find_until_run() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let s = waiting(
        vec![cfg],
        vec![tab(id, "users"), tab(id, "orders")],
        Some(0),
    );
    let (s, _) = connected(s, id);
    let (mut s, commands) = update(s, Input::Tabs(TabAction::Next));
    assert!(
        commands.is_empty(),
        "showing a tab sends nothing: {commands:?}"
    );
    let (s2, commands) = update(s, Input::Tabs(TabAction::Previous));
    assert!(commands.is_empty(), "{commands:?}");
    s = s2;
    s.explain.guard = false;
    let tab = s.active_tab().expect("a tab");
    let (s, commands) = update(s, Input::Tabs(TabAction::Run(tab)));
    assert_eq!(finds(&commands).len(), 1, "{commands:?}");
    assert_eq!(s.freshness_of(tab), Freshness::Fresh);
}

#[test]
fn a_restored_draft_with_out_stage_sends_nothing_and_run_still_needs_typed_confirmation() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let draft = SavedTab {
        sub_tab: SavedSubTab::Aggregate,
        find: None,
        draft: Some(SavedDraftText {
            stages: vec![stage(r#"{"$match": {}}"#), stage(r#"{"$out": "dest"}"#)],
        }),
        ..tab(id, "users")
    };
    let s = waiting(vec![cfg], vec![draft], Some(0));
    let (s, commands) = connected(s, id);
    only_list_databases(id, &commands);
    assert_eq!(s.tab, CollectionTab::Aggregate);
    assert_eq!(s.aggregate.stages.len(), 2);
    assert!(
        !s.aggregate.wants_auto_refresh(s.aggregate_view.auto_sent),
        "no preview is asked for on its own"
    );
    assert!(s.aggregate.in_flight.is_none());
    // Run goes through the typed confirmation: confirming without it sends nothing.
    let (s, _) = update(s, Input::Aggregate(AggregateAction::RunClicked));
    let (s, commands) = update(s, Input::Aggregate(AggregateAction::RunConfirmed));
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, Command::AggregateRun { .. }))
    );
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::RunTyped("dest".into())),
    );
    let (_, commands) = update(s, Input::Aggregate(AggregateAction::RunConfirmed));
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::AggregateRun { .. })),
        "typed, it runs: {commands:?}"
    );
}

#[test]
fn a_restored_indexes_tab_lists_indexes_on_run_only() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let indexes = SavedTab {
        sub_tab: SavedSubTab::Indexes,
        ..tab(id, "users")
    };
    let s = waiting(vec![cfg], vec![indexes], Some(0));
    let (mut s, commands) = connected(s, id);
    only_list_databases(id, &commands);
    assert_eq!(s.tab, CollectionTab::Indexes);
    s.explain.guard = false;
    let tab = s.active_tab().expect("a tab");
    let (_, commands) = update(s, Input::Tabs(TabAction::Run(tab)));
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ListIndexes { ns, .. } if ns.collection == "users")),
        "{commands:?}"
    );
}

#[test]
fn a_failed_connect_keeps_tabs_dormant_and_shows_the_failed_root() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let s = waiting(vec![cfg], vec![tab(id, "users")], Some(0));
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    let (s, _) = update(
        s,
        Input::Event(Event::ConnectFailed {
            session: id,
            epoch,
            error: curlybrace_core::error::CoreError::Unreachable,
        }),
    );
    assert_eq!(s.status, ConnStatus::Failed);
    assert_eq!(s.restore.dormant[&id].len(), 1, "still waiting");
    assert!(s.visible_tabs().iter().all(|t| s.tab_ns(*t).is_none()));
    // A later connect brings them.
    let (s, commands) = connected(s, id);
    only_list_databases(id, &commands);
    assert_eq!(namespaces(&s, id), ["users"]);
}

#[test]
fn a_deleted_connection_drops_its_dormant_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (
        ConnectionConfig::new("a", "mongodb://a"),
        ConnectionConfig::new("b", "mongodb://b"),
    );
    let (ida, idb) = (a.id, b.id);
    let mut s = waiting(
        vec![a, b],
        vec![tab(ida, "users"), tab(idb, "orders")],
        Some(0),
    );
    assert!(s.restore.active_hint.is_some());
    let secrets = MemorySecretStore::default();
    let path = Ok(dir.path().join("connections.json"));
    s = crate::app::connections::deleted(s, ida, &secrets, &path).0;
    assert!(!s.restore.dormant.contains_key(&ida));
    assert!(s.restore.dormant.contains_key(&idb));
    assert_eq!(s.restore.active_hint, None, "its active tab went with it");
}

#[test]
fn restored_tab_on_production_connection_carries_the_badge() {
    let mut cfg = ConnectionConfig::new("prod", "mongodb://localhost");
    cfg.production = true;
    let id = cfg.id;
    let s = waiting(
        vec![cfg],
        vec![tab(id, "users"), tab(id, "orders")],
        Some(0),
    );
    let (s, _) = connected(s, id);
    let shown = labels(&s, &crate::I18n::english());
    assert_eq!(shown.len(), 2);
    for (_, label) in shown {
        assert!(label.badge.production, "{label:?}");
        assert!(label.dormant);
    }
    // A write from a restored tab is still held for the production confirmation.
    let editor = EditorAction::Save {
        original_id: None,
        text: "{}".into(),
    };
    let (s, commands) = update(s, Input::Editor(editor));
    assert!(s.write_warning.is_some(), "parked for confirmation");
    assert!(!commands.iter().any(|c| matches!(c, Command::Insert { .. })));
}

#[test]
fn restored_tab_on_read_only_connection_refuses_writes() {
    let mut cfg = ConnectionConfig::new("ro", "mongodb://localhost");
    cfg.read_only = true;
    let id = cfg.id;
    let s = waiting(vec![cfg], vec![tab(id, "users")], Some(0));
    let (s, _) = connected(s, id);
    let shown = labels(&s, &crate::I18n::english());
    assert!(!shown.is_empty() && shown.iter().all(|(_, l)| l.badge.read_only));
    let editor = EditorAction::Save {
        original_id: None,
        text: "{}".into(),
    };
    let (s, commands) = update(s, Input::Editor(editor));
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(commands.is_empty(), "{commands:?}");
}

#[test]
fn max_tabs_respected_when_materialising_on_top_of_open_tabs() {
    let (a, b) = (
        ConnectionConfig::new("a", "mongodb://a"),
        ConnectionConfig::new("b", "mongodb://b"),
    );
    let (ida, idb) = (a.id, b.id);
    let saved: Vec<SavedTab> = (0..5).map(|n| tab(idb, &format!("saved{n}"))).collect();
    let s = waiting(vec![a, b], saved, None);
    let (mut s, _) = connected(s, ida);
    for n in 0..MAX_TABS - 1 {
        let ns = Namespace::new("app", format!("open{n}"));
        s = update(s, Input::Tabs(TabAction::OpenInNew { session: ida, ns })).0;
    }
    assert_eq!(s.visible_tabs().len(), MAX_TABS - 1);
    let (s, _) = connected(s, idb);
    assert_eq!(namespaces(&s, idb).len(), 1, "only the room that was left");
    let held: usize = s
        .visible_tabs()
        .into_iter()
        .filter(|t| s.tab_ns(*t).is_some())
        .count();
    assert_eq!(held, MAX_TABS);
}

#[test]
fn text_dropped_tab_shows_its_note() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let dropped = SavedTab {
        find: None,
        text_dropped: true,
        ..tab(id, "users")
    };
    let s = waiting(vec![cfg], vec![dropped, tab(id, "orders")], Some(0));
    let (s, _) = connected(s, id);
    let users = restored_tab(&s, "users");
    let orders = restored_tab(&s, "orders");
    assert!(s.tabs.text_dropped.contains(&users));
    assert!(!s.tabs.text_dropped.contains(&orders));
    assert!(s.tabs.restored.contains(&users) && s.tabs.restored.contains(&orders));
    // Run wakes the tab: the note and the restored mark go.
    let (mut s, _) = update(s, Input::Tabs(TabAction::Activate(users)));
    s.explain.guard = false;
    let (s, _) = update(s, Input::Tabs(TabAction::Run(users)));
    assert!(!s.tabs.text_dropped.contains(&users) && !s.tabs.restored.contains(&users));
}

#[test]
fn the_saved_split_comes_back_with_its_panes() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let file = WorkspaceFile {
        tabs: vec![tab(id, "users"), tab(id, "orders")],
        active: Some(0),
        split: Some(crate::app::workspace_file::SavedSplit {
            secondary: 1,
            ratio: 0.4,
            orientation: crate::app::workspace_file::SavedOrientation::Stacked,
            secondary_first: false,
        }),
        ..WorkspaceFile::default()
    };
    let mut s = AppState::new(vec![cfg]);
    s.restore = Box::new(RestoreState::of_file(&file, WorkspaceSync::default()));
    let (s, commands) = connected(s, id);
    only_list_databases(id, &commands);
    let split = s.tabs.split.clone().expect("a split");
    assert_eq!(Some(split.primary), s.active_tab());
    assert_eq!(
        s.tab_ns(split.secondary).map(|n| n.collection),
        Some("orders".into())
    );
    assert!((split.ratio - 0.4).abs() < 1e-6);
}

#[test]
fn a_connection_behind_another_one_takes_nothing_from_the_front() {
    let (a, b) = (
        ConnectionConfig::new("a", "mongodb://a"),
        ConnectionConfig::new("b", "mongodb://b"),
    );
    let (ida, idb) = (a.id, b.id);
    let s = waiting(vec![a, b], vec![tab(idb, "orders")], Some(0));
    let (s, _) = connected(s, ida);
    let (s, _) = update(s, Input::Connect(idb));
    let epoch = s.epoch_counter;
    // The person goes back to `a` while `b` is still connecting.
    let (s, _) = update(s, Input::Activate(ida));
    let (s, commands) = update(
        s,
        Input::Event(Event::Connected {
            session: idb,
            epoch,
            topology: None,
        }),
    );
    only_list_databases(idb, &commands);
    assert_eq!(s.active, Some(ida), "focus stays");
    assert_eq!(namespaces(&s, idb), ["orders"], "the tab is there, behind");
}
