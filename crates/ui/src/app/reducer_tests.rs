use super::{Input, apply_event, update};
use crate::app::persistence::PersistError;
use crate::app::state::{AppState, Banner, ConnStatus, EditorKey, HostEndpoint, Pending};
use crate::viewmodel::{DbNode, DocAction, EditorAction, EditorState, FindAction, SidebarAction};
use crate::viewmodel::{PageKey, page_key};
use bson::{Bson, Document, doc};
use curlybrace_core::connection::{ConnectionConfig, SshAuth, SshConfig};
use curlybrace_core::error::CoreError;
use curlybrace_core::query::QueryText;
use curlybrace_core::runtime::EMBEDDED_SESSION;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use curlybrace_core::test_support::mock_objects::mock_doc_with_id;

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

fn page_of(docs: Vec<Document>, has_more: bool, skip: u64) -> Page {
    Page {
        docs,
        skip,
        has_more,
    }
}

fn three_docs() -> Page {
    page_of((1..=3).map(mock_doc_with_id).collect(), false, 0)
}

/// Connected, `app` expanded and loaded, `app.people` selected showing three docs.
fn browsing() -> AppState {
    let mut s = AppState::new(vec![]);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into(), "other".into()])
        .with_collections("app", vec!["people".into(), "pets".into()])
        .toggled("app")
        .selecting(ns());
    let page = three_docs();
    s.page_key = Some(page_key(&page));
    s.page = Some(page);
    s
}

fn run(state: AppState, input: Input) -> (AppState, Vec<Command>) {
    update(state, input)
}

fn event(state: AppState, ev: Event) -> (AppState, Vec<Command>) {
    run(state, Input::Event(ev))
}

fn node<'a>(s: &'a AppState, name: &str) -> &'a DbNode {
    s.sidebar.databases.iter().find(|d| d.name == name).unwrap()
}

fn ssh_cfg() -> ConnectionConfig {
    let mut c = ConnectionConfig::new("via bastion", "mongodb://db:27017");
    c.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 2222,
        username: "me".into(),
        auth: SshAuth::Password,
    });
    c
}

/// A mutation in flight in `app.people`, saved from `editor` when given.
fn pending_from(editor: Option<&EditorState>) -> Option<Pending> {
    Some(Pending {
        ns: ns(),
        editor: editor.map(|e| EditorKey {
            original_id: e.original_id.clone(),
            text: e.text.clone(),
        }),
    })
}

fn find_failed() -> Event {
    Event::Failed {
        session: EMBEDDED_SESSION,
        context: "find",
        error: CoreError::Database("x".into()),
    }
}

// ---- events ----

#[test]
fn connected_sets_status_and_lists_databases() {
    let mut s = AppState::new(vec![]);
    s.status = ConnStatus::Connecting;
    let (s, cmds) = event(
        s,
        Event::Connected {
            session: EMBEDDED_SESSION,
            epoch: 1,
            topology: None,
        },
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(matches!(cmds.as_slice(), [Command::ListDatabases { .. }]));
}

#[test]
fn databases_fill_the_sidebar_keeping_loaded_state() {
    let (s, cmds) = event(
        browsing(),
        Event::Databases {
            session: EMBEDDED_SESSION,
            names: vec!["app".into(), "new".into()],
        },
    );
    assert!(cmds.is_empty());
    let names: Vec<_> = s
        .sidebar
        .databases
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(names, ["app", "new"]);
    assert!(node(&s, "app").collections.is_some());
}

#[test]
fn collections_are_stored_and_clear_the_in_flight_mark() {
    let mut s = browsing();
    s.loading.insert("other".into());
    let (s, _) = event(
        s,
        Event::Collections {
            session: EMBEDDED_SESSION,
            db: "other".into(),
            names: vec!["c".into()],
        },
    );
    assert_eq!(node(&s, "other").collections, Some(vec!["c".to_string()]));
    assert!(s.loading.is_empty());
}

#[test]
fn results_replace_page_bump_tree_and_memoise_key() {
    let s = browsing();
    let generation = s.tree.generation();
    let q = s.issued_query.clone();
    let page = page_of(vec![mock_doc_with_id(9_i32)], true, 0);
    let (s, cmds) = event(
        s,
        Event::Results {
            session: EMBEDDED_SESSION,
            ns: ns(),
            query: q,
            page: page.clone(),
        },
    );
    assert!(cmds.is_empty());
    assert_eq!(s.page, Some(page.clone()));
    assert_eq!(s.page_key, Some(page_key(&page)));
    assert_eq!(s.tree.generation(), generation + 1);
}

#[test]
fn results_for_another_namespace_are_ignored() {
    let s = browsing();
    let before = s.page.clone();
    let (s, _) = event(
        s,
        Event::Results {
            session: EMBEDDED_SESSION,
            ns: Namespace::new("app", "pets"),
            query: Query::default(),
            page: page_of(vec![], false, 0),
        },
    );
    assert_eq!(s.page, before);
}

#[test]
fn results_for_an_older_query_are_ignored() {
    let mut s = browsing();
    s.issued_query = Query {
        skip: 50,
        ..Query::default()
    };
    let before = s.page.clone();
    let (s, _) = event(
        s,
        Event::Results {
            session: EMBEDDED_SESSION,
            ns: ns(),
            query: Query::default(),
            page: page_of(vec![], false, 0),
        },
    );
    assert_eq!(s.page, before);
}

#[test]
fn failed_find_keeps_page_query_text_and_sets_a_banner() {
    let mut s = browsing();
    s.find.text = QueryText {
        filter: "{\"a\": 1}".into(),
        ..QueryText::default()
    };
    let (page, key, text) = (s.page.clone(), s.page_key, s.find.text.clone());
    let (s, cmds) = event(s, find_failed());
    assert!(cmds.is_empty());
    assert_eq!((s.page, s.page_key, s.find.text), (page, key, text));
    assert_eq!(
        s.banner,
        Some(Banner::Failure {
            context: "find",
            error: CoreError::Database("x".into())
        })
    );
}

#[test]
fn newer_banner_replaces_older_and_dismiss_clears() {
    let (s, _) = event(browsing(), find_failed());
    let (s, _) = event(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "list_databases",
            error: CoreError::NotFound,
        },
    );
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "list_databases",
            ..
        })
    ));
    let (s, _) = run(s, Input::DismissBanner);
    assert_eq!(s.banner, None);
}

#[test]
fn failed_list_collections_clears_in_flight_and_banners() {
    let mut s = browsing();
    s.loading.insert("other".into());
    let (s, _) = event(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "list_collections",
            error: CoreError::NotFound,
        },
    );
    assert!(s.loading.is_empty());
    assert!(s.banner.is_some());
}

#[test]
fn mutated_reruns_the_page_query_and_closes_the_editor_it_came_from() {
    let mut s = browsing();
    s.page_query = Query {
        skip: 50,
        ..Query::default()
    };
    let editor = EditorState::for_new();
    s.pending = pending_from(Some(&editor));
    s.editor = Some(editor);
    let (s, cmds) = event(
        s,
        Event::Mutated {
            session: EMBEDDED_SESSION,
            ns: ns(),
        },
    );
    assert!(s.pending.is_none() && s.editor.is_none());
    match cmds.as_slice() {
        [Command::Find { ns: n, query, .. }] => {
            assert_eq!((n, query.skip), (&ns(), 50));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn mutated_leaves_an_editor_with_other_content_open() {
    let mut s = browsing();
    s.pending = pending_from(Some(&EditorState::for_new()));
    let mut other = EditorState::for_new();
    other.text = "{\"typing\": true}".into();
    s.editor = Some(other);
    let (s, _) = event(
        s,
        Event::Mutated {
            session: EMBEDDED_SESSION,
            ns: ns(),
        },
    );
    assert!(s.editor.is_some());
}

#[test]
fn mutated_for_another_namespace_neither_refetches_nor_clears_pending() {
    let mut s = browsing();
    s.pending = pending_from(None);
    let (s, cmds) = event(
        s,
        Event::Mutated {
            session: EMBEDDED_SESSION,
            ns: Namespace::new("app", "pets"),
        },
    );
    assert!(cmds.is_empty() && s.pending.is_some());
}

#[test]
fn open_keeps_a_pending_mutation_so_a_second_submit_is_still_refused() {
    let mut s = browsing();
    s.pending = pending_from(None);
    let (s, _) = run(
        s,
        Input::Sidebar(SidebarAction::Open(Namespace::new("app", "pets"))),
    );
    assert!(s.pending.is_some());
    let (s, cmds) = run(s, Input::Doc(DocAction::Insert));
    let (_, cmds2) = run(
        s,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text: "{}".into(),
        }),
    );
    assert!(cmds.is_empty() && cmds2.is_empty());
}

#[test]
fn failed_mutation_keeps_the_editor_open_with_the_error() {
    let mut s = browsing();
    let editor = EditorState::for_new();
    s.pending = pending_from(Some(&editor));
    s.editor = Some(editor);
    let (s, _) = event(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "insert",
            error: CoreError::Database("dup".into()),
        },
    );
    assert!(s.pending.is_none());
    let editor = s.editor.expect("editor stays open");
    assert_eq!(editor.error, Some(CoreError::Database("dup".into())));
    assert!(s.banner.is_none());
}

#[test]
fn failed_save_goes_to_a_banner_when_the_editor_changed_meanwhile() {
    let mut s = browsing();
    s.pending = pending_from(Some(&EditorState::for_new()));
    let mut other = EditorState::for_edit(&mock_doc_with_id(2_i32));
    other.error = None;
    s.editor = Some(other.clone());
    let (s, _) = event(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "replace",
            error: CoreError::Database("x".into()),
        },
    );
    assert_eq!(s.editor, Some(other));
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "replace",
            ..
        })
    ));
    assert!(s.pending.is_none());
}

#[test]
fn failed_delete_sets_a_banner_and_clears_pending() {
    let mut s = browsing();
    s.pending = pending_from(None);
    let (s, _) = event(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "delete",
            error: CoreError::NotFound,
        },
    );
    assert!(s.pending.is_none() && s.banner.is_some());
}

#[test]
fn disconnected_clears_session_state() {
    let mut s = browsing();
    s.editor = Some(EditorState::for_new());
    let (s, _) = event(
        s,
        Event::Disconnected {
            session: EMBEDDED_SESSION,
            epoch: 1,
        },
    );
    assert_eq!(s.status, ConnStatus::Disconnected);
    assert!(s.sidebar.databases.is_empty() && s.sidebar.selected.is_none());
    assert!(s.page.is_none() && s.page_key.is_none() && s.editor.is_none());
}

#[test]
fn failed_connect_leaves_a_disconnected_state_with_a_banner() {
    let mut s = browsing();
    s.status = ConnStatus::Connecting;
    let (s, _) = event(
        s,
        Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            epoch: 1,
            error: CoreError::Ssh("x".into()),
        },
    );
    assert_eq!(s.status, ConnStatus::Disconnected);
    assert!(s.sidebar.databases.is_empty() && s.page.is_none());
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "connect",
            ..
        })
    ));
}

#[test]
fn failed_disconnect_also_ends_up_disconnected() {
    let s = browsing();
    let (s, _) = event(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "disconnect",
            error: CoreError::Database("x".into()),
        },
    );
    assert_eq!(s.status, ConnStatus::Disconnected);
}

#[test]
fn data_events_after_a_disconnect_are_ignored() {
    let mut s = browsing();
    s.status = ConnStatus::Disconnected;
    let (s, _) = event(s, find_failed());
    assert!(s.banner.is_none());
    let (s, cmds) = event(
        s,
        Event::Mutated {
            session: EMBEDDED_SESSION,
            ns: ns(),
        },
    );
    assert!(cmds.is_empty() && s.page.is_some());
}

#[test]
fn apply_event_is_the_state_only_view_of_update() {
    let s = apply_event(
        browsing(),
        Event::Databases {
            session: EMBEDDED_SESSION,
            names: vec!["x".into()],
        },
    );
    assert_eq!(s.sidebar.databases.len(), 1);
}

// ---- connect / disconnect ----

#[test]
fn connect_sends_the_saved_config_and_remembers_the_ssh_endpoint() {
    let cfg = ssh_cfg();
    let id = cfg.id;
    let mut s = AppState::new(vec![cfg.clone()]);
    s.banner = Some(Banner::NotConnected);
    let (s, cmds) = run(s, Input::Connect(id));
    assert_eq!(s.status, ConnStatus::Connecting);
    assert_eq!(s.banner, None);
    assert_eq!(
        s.ssh_endpoint,
        Some(HostEndpoint {
            host: "bastion".into(),
            port: 2222
        })
    );
    assert!(matches!(cmds.as_slice(), [Command::Connect(c)] if *c == cfg));
}

#[test]
fn connect_to_an_unknown_id_does_nothing() {
    let (s, cmds) = run(AppState::new(vec![]), Input::Connect(uuid::Uuid::new_v4()));
    assert!(cmds.is_empty() && s.status == ConnStatus::Disconnected);
}

#[test]
fn disconnect_clears_the_session_right_away_and_sends_disconnect() {
    let (s, cmds) = run(browsing(), Input::Disconnect);
    assert_eq!(s.status, ConnStatus::Disconnected);
    assert!(s.page.is_none() && s.sidebar.databases.is_empty());
    assert!(matches!(cmds.as_slice(), [Command::Disconnect { .. }]));
}

// ---- sidebar ----

#[test]
fn load_collections_expands_and_sends_once() {
    let mut s = browsing();
    s.sidebar = s.sidebar.with_databases(vec!["app".into(), "other".into()]);
    let (s, cmds) = run(
        s,
        Input::Sidebar(SidebarAction::LoadCollections("other".into())),
    );
    assert!(node(&s, "other").expanded);
    assert!(s.loading.contains("other"));
    assert!(matches!(cmds.as_slice(), [Command::ListCollections { db, .. }] if db == "other"));
    // A second request while in flight expands nothing new and sends nothing.
    let (s, cmds) = run(
        s,
        Input::Sidebar(SidebarAction::LoadCollections("other".into())),
    );
    assert!(cmds.is_empty());
    assert!(node(&s, "other").expanded);
}

#[test]
fn toggle_only_toggles() {
    let (s, cmds) = run(
        browsing(),
        Input::Sidebar(SidebarAction::Toggle("app".into())),
    );
    assert!(cmds.is_empty() && !node(&s, "app").expanded);
}

#[test]
fn open_selects_resets_paging_and_finds_the_default_query() {
    let mut s = browsing();
    s.issued_query = Query {
        skip: 100,
        ..Query::default()
    };
    s.page_query = s.issued_query.clone();
    s.find.text.filter = "{\"a\":1}".into();
    s.editor = Some(EditorState::for_new());
    let target = Namespace::new("app", "pets");
    let (s, cmds) = run(s, Input::Sidebar(SidebarAction::Open(target.clone())));
    assert_eq!(s.sidebar.selected, Some(target.clone()));
    assert_eq!(s.issued_query, Query::default());
    assert_eq!(s.page_query, Query::default());
    assert!(s.loading_page);
    assert_eq!(s.find.text, QueryText::default());
    assert!(s.editor.is_none() && s.page.is_none() && s.page_key.is_none());
    match cmds.as_slice() {
        [Command::Find { ns: n, query, .. }] => {
            assert_eq!((n, query), (&target, &Query::default()))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn open_resets_a_pending_delete_confirmation() {
    let mut s = browsing();
    let key: PageKey = s.page_key.unwrap();
    s.tree.confirm = crate::viewmodel::DeleteConfirm::Pending {
        id: Bson::Int32(1),
        page: key,
    };
    let (s, _) = run(
        s,
        Input::Sidebar(SidebarAction::Open(Namespace::new("app", "pets"))),
    );
    assert_eq!(s.tree.confirm, crate::viewmodel::DeleteConfirm::None);
}

#[test]
fn refresh_lists_databases_and_forgets_loaded_collections() {
    let mut s = browsing();
    s.loading.insert("app".into());
    let (s, cmds) = run(s, Input::Sidebar(SidebarAction::Refresh));
    assert!(matches!(cmds.as_slice(), [Command::ListDatabases { .. }]));
    assert!(s.loading.is_empty());
    assert!(
        s.sidebar
            .databases
            .iter()
            .all(|d| d.collections.is_none() && !d.expanded)
    );
}

// ---- find bar ----

#[test]
fn run_stores_the_query_and_sends_find() {
    let q = Query {
        filter: doc! {"name": "alice"},
        ..Query::default()
    };
    let (s, cmds) = run(browsing(), Input::Find(FindAction::Run(q.clone())));
    assert_eq!(s.issued_query, q);
    assert!(matches!(cmds.as_slice(), [Command::Find { query, .. }] if *query == q));
}

fn results_for(s: &AppState, has_more: bool) -> Event {
    Event::Results {
        session: EMBEDDED_SESSION,
        ns: ns(),
        query: s.issued_query.clone(),
        page: page_of(vec![mock_doc_with_id(1_i32)], has_more, s.issued_query.skip),
    }
}

#[test]
fn next_and_prev_page_from_the_displayed_pages_query() {
    let mut s = browsing();
    s.page = Some(page_of(vec![mock_doc_with_id(1_i32)], true, 0));
    let (s, cmds) = run(s, Input::Find(FindAction::Next));
    assert_eq!(s.issued_query.skip, 50);
    assert!(matches!(cmds.as_slice(), [Command::Find { query, .. }] if query.skip == 50));
    let ev = results_for(&s, false);
    let (s, _) = event(s, ev);
    assert_eq!(s.page_query.skip, 50);
    let (s, cmds) = run(s, Input::Find(FindAction::Prev));
    assert_eq!(s.issued_query.skip, 0);
    assert!(matches!(cmds.as_slice(), [Command::Find { query, .. }] if query.skip == 0));
}

#[test]
fn failed_find_keeps_page_query_so_paging_and_has_prev_still_follow_the_page() {
    let mut s = browsing();
    s.page = Some(page_of(vec![mock_doc_with_id(1_i32)], true, 50));
    s.page_query = Query {
        skip: 50,
        ..Query::default()
    };
    let failed = Query {
        filter: doc! {"x": 1},
        ..Query::default()
    };
    let (s, _) = run(s, Input::Find(FindAction::Run(failed.clone())));
    assert_eq!(s.issued_query, failed);
    let (s, _) = event(s, find_failed());
    assert_eq!(s.page_query.skip, 50);
    assert_eq!(s.paging(), (true, true), "has_prev derives from page_query");
    let (s, cmds) = run(s, Input::Find(FindAction::Next));
    assert!(matches!(
        cmds.as_slice(),
        [Command::Find { query, .. }] if query.skip == 100 && query.filter.is_empty()
    ));
    let (_, cmds) = run(s, Input::Find(FindAction::Prev));
    assert!(matches!(cmds.as_slice(), [Command::Find { query, .. }] if query.skip == 0));
}

#[test]
fn failed_find_then_an_edit_refetches_the_displayed_pages_query() {
    let mut s = browsing();
    let failed = Query {
        filter: doc! {"x": 1},
        ..Query::default()
    };
    s.pending = pending_from(None);
    let (s, _) = run(s, Input::Find(FindAction::Run(failed)));
    let (s, _) = event(s, find_failed());
    let (s, cmds) = event(
        s,
        Event::Mutated {
            session: EMBEDDED_SESSION,
            ns: ns(),
        },
    );
    match cmds.as_slice() {
        [Command::Find { query, .. }] => assert_eq!(*query, Query::default()),
        other => panic!("{other:?}"),
    }
    // The re-fetch is what results are matched against now, so the edited page is accepted.
    assert_eq!(s.issued_query, Query::default());
    let edited = page_of(vec![doc! {"_id": 1_i32, "name": "edited"}], false, 0);
    let (s, _) = event(
        s,
        Event::Results {
            session: EMBEDDED_SESSION,
            ns: ns(),
            query: Query::default(),
            page: edited.clone(),
        },
    );
    assert_eq!(s.page, Some(edited));
}

#[test]
fn failed_first_find_after_open_stops_loading_and_keeps_the_text() {
    let (s, _) = run(
        browsing(),
        Input::Sidebar(SidebarAction::Open(Namespace::new("app", "pets"))),
    );
    assert!(s.loading_page && s.page.is_none());
    let (s, _) = event(s, find_failed());
    assert!(!s.loading_page && s.page.is_none() && s.banner.is_some());
}

#[test]
fn clear_runs_the_default_query() {
    let mut s = browsing();
    s.issued_query = Query {
        skip: 50,
        filter: doc! {"a": 1},
        ..Query::default()
    };
    let (s, cmds) = run(s, Input::Find(FindAction::Clear));
    assert_eq!(s.issued_query, Query::default());
    assert!(matches!(cmds.as_slice(), [Command::Find { query, .. }] if *query == Query::default()));
}

#[test]
fn paging_flags_follow_skip_and_has_more() {
    let mut s = browsing();
    assert_eq!(s.paging(), (false, false));
    s.page_query.skip = 50;
    s.page = Some(page_of(vec![], true, 50));
    assert_eq!(s.paging(), (true, true));
}

#[test]
fn data_commands_without_a_connection_are_dropped_with_a_banner() {
    let mut s = browsing();
    s.status = ConnStatus::Disconnected;
    for input in [
        Input::Find(FindAction::Run(Query::default())),
        Input::Find(FindAction::Clear),
        Input::Sidebar(SidebarAction::Refresh),
        Input::Sidebar(SidebarAction::Open(ns())),
        Input::Sidebar(SidebarAction::LoadCollections("app".into())),
        Input::Doc(DocAction::Clone(mock_doc_with_id(1_i32))),
        Input::Doc(DocAction::Delete(Bson::Int32(1))),
    ] {
        let label = format!("{input:?}");
        let mut state = browsing();
        state.status = ConnStatus::Disconnected;
        let (state, cmds) = run(state, input);
        assert!(cmds.is_empty(), "{label}");
        assert_eq!(state.banner, Some(Banner::NotConnected), "{label}");
    }
    let _ = s;
}

// ---- documents and editor ----

#[test]
fn edit_and_insert_open_the_editor() {
    let d = mock_doc_with_id(2_i32);
    let (s, _) = run(browsing(), Input::Doc(DocAction::Edit(d.clone())));
    assert_eq!(s.editor, Some(EditorState::for_edit(&d)));
    let (s, _) = run(browsing(), Input::Doc(DocAction::Insert));
    assert_eq!(s.editor, Some(EditorState::for_new()));
}

#[test]
fn clone_sends_the_source_document() {
    let d = mock_doc_with_id(2_i32);
    let (s, cmds) = run(browsing(), Input::Doc(DocAction::Clone(d.clone())));
    assert!(s.pending.is_some());
    assert!(
        matches!(cmds.as_slice(), [Command::Clone { ns: n, source, .. }] if *n == ns() && *source == d)
    );
}

#[test]
fn delete_of_a_document_on_the_page_sends_delete() {
    let (s, cmds) = run(browsing(), Input::Doc(DocAction::Delete(Bson::Int32(2))));
    assert!(s.pending.is_some());
    assert!(matches!(cmds.as_slice(), [Command::Delete { id, .. }] if *id == Bson::Int32(2)));
}

#[test]
fn delete_of_an_id_not_on_the_current_page_is_refused() {
    let (s, cmds) = run(browsing(), Input::Doc(DocAction::Delete(Bson::Int32(99))));
    assert!(cmds.is_empty() && s.pending.is_none());
}

fn save(id: Option<Bson>, text: &str) -> Input {
    Input::Editor(EditorAction::Save {
        original_id: id,
        text: text.into(),
    })
}

#[test]
fn save_with_an_id_replaces_and_without_inserts() {
    let (_, cmds) = run(browsing(), save(Some(Bson::Int32(1)), "{\"_id\":1}"));
    assert!(
        matches!(cmds.as_slice(), [Command::Replace { id, json, .. }]
        if *id == Bson::Int32(1) && json == "{\"_id\":1}")
    );
    let (_, cmds) = run(browsing(), save(None, "{}"));
    assert!(matches!(cmds.as_slice(), [Command::Insert { json, .. }] if json == "{}"));
}

#[test]
fn second_save_while_a_mutation_is_pending_sends_nothing() {
    let (s, first) = run(browsing(), save(None, "{}"));
    assert_eq!(first.len(), 1);
    let (s, second) = run(s, save(None, "{}"));
    assert!(second.is_empty() && s.pending.is_some());
}

#[test]
fn cancel_closes_the_editor_without_commands() {
    let mut s = browsing();
    s.editor = Some(EditorState::for_new());
    let (s, cmds) = run(s, Input::Editor(EditorAction::Cancel));
    assert!(s.editor.is_none() && cmds.is_empty());
}

#[path = "reducer_session_tests.rs"]
mod session;

#[path = "reducer_epoch_tests.rs"]
mod epoch;
