//! Connection manager wiring: the real runtime (fake connector), a memory keychain and a config
//! file in a temp dir, driven through the app's own action entry point.
mod conn_harness;

use conn_harness::{FixedPath, Harness, names, people};
use curlybrace_core::runtime::TestReport;
use curlybrace_core::test_support::mock_objects::{
    MOCK_PASSWORD_MARKER, mock_connection, mock_connections,
};
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::ConnStatus;
use curlybrace_ui::viewmodel::{
    ConnectionForm, SidebarAction, Sort, connection_list::grouped, connection_list::visible,
};
use curlybrace_ui::views::connections::{ConnectionAction, Notice, TestState};
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_copies_the_secret_and_never_writes_it_to_the_file() {
    let source = mock_connection();
    let id = source.id;
    let mut h = Harness::new(vec![source]);
    h.store_password(id, MOCK_PASSWORD_MARKER);

    h.app.dispatch_connection(ConnectionAction::Duplicate(id));

    let list = &h.app.state().conn.connections;
    assert_eq!(list.len(), 2, "two rows");
    let copy = list.iter().find(|c| c.id != id).expect("the copy");
    assert_ne!(copy.name, list.iter().find(|c| c.id == id).unwrap().name);
    assert_eq!(
        h.password_of(copy.id).as_deref(),
        Some(MOCK_PASSWORD_MARKER)
    );
    assert_eq!(h.password_of(id).as_deref(), Some(MOCK_PASSWORD_MARKER));
    assert_eq!(h.saved().len(), 2, "the file holds both");
    assert!(!h.saved_text().contains(MOCK_PASSWORD_MARKER));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_connection_while_connected_leaves_the_browse_session_usable() {
    let (a, b) = (mock_connection(), mock_connections().remove(1));
    let (id_a, id_b) = (a.id, b.id);
    let mut h = Harness::new(vec![a, b]);
    h.app.dispatch(Input::Connect(id_a));
    h.until("connected and listed", |a| {
        a.state().status == ConnStatus::Connected && !a.state().sidebar.databases.is_empty()
    });

    let form = ConnectionForm::from_config(&h.app.state().connection(id_b).unwrap().clone());
    h.app
        .dispatch_connection(ConnectionAction::Test(Box::new(form)));
    h.until("test result", |a| {
        matches!(a.state().conn.test, TestState::Done { .. })
    });
    match &h.app.state().conn.test {
        TestState::Done { outcome, .. } => {
            assert!(matches!(outcome, Ok(TestReport { .. })), "{outcome:?}");
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(h.app.state().status, ConnStatus::Connected);
    assert_eq!(h.app.state().connected, Some(id_a));

    // The session still answers: expand and open still work after the test.
    h.app
        .dispatch(Input::Sidebar(SidebarAction::LoadCollections(
            "testdb".into(),
        )));
    h.until("collections", |a| {
        a.state().sidebar.databases[0].collections.is_some()
    });
    h.app
        .dispatch(Input::Sidebar(SidebarAction::Open(people())));
    h.until("page", |a| {
        a.state().page.as_ref().is_some_and(|p| p.docs.len() == 1)
    });
    assert_eq!(h.app.state().banner, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_then_import_into_a_fresh_app_keeps_names_with_new_ids() {
    let original = mock_connections();
    let mut first = Harness::new(original.clone());
    let exported = first.dir.path().join("export.json");
    first.app = first
        .app
        .with_file_picker(Arc::new(FixedPath(exported.clone())), Arc::new(|| {}));
    first.app.dispatch_connection(ConnectionAction::StartExport);
    first.until("exported", |a| {
        matches!(a.state().conn.notice, Some(Notice::Exported { count: 3 }))
    });
    assert!(
        !std::fs::read_to_string(&exported)
            .unwrap()
            .contains(MOCK_PASSWORD_MARKER)
    );

    let mut second = Harness::new(Vec::new());
    second.app = second
        .app
        .with_file_picker(Arc::new(FixedPath(exported)), Arc::new(|| {}));
    second
        .app
        .dispatch_connection(ConnectionAction::StartImport);
    second.until("imported", |a| {
        matches!(
            a.state().conn.notice,
            Some(Notice::Imported {
                imported: 3,
                skipped: 0
            })
        )
    });

    let got = &second.app.state().conn.connections;
    assert_eq!(names(got), names(&original));
    for (new, old) in got.iter().zip(&original) {
        assert_ne!(new.id, old.id, "{} got a fresh id", new.name);
        assert_eq!(new.folder, old.folder);
    }
    assert_eq!(names(&second.saved()), names(&original), "and it was saved");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_keeps_folders_timestamps_and_the_last_connected_stamp() {
    let mut h = Harness::new(Vec::new());
    for cfg in mock_connections() {
        let mut form = ConnectionForm::from_config(&cfg);
        form.password = String::new();
        h.app
            .dispatch_connection(ConnectionAction::Save(Box::new(form)));
    }
    let target = h.app.state().conn.connections[1].id;
    h.app.dispatch(Input::Connect(target));
    h.until("connected", |a| a.state().status == ConnStatus::Connected);

    let before = h.app.state().conn.connections.clone();
    let stamped = before.iter().find(|c| c.id == target).unwrap();
    assert!(stamped.last_connected.is_some());
    assert!(stamped.created_at.is_some() && stamped.updated_at.is_some());

    // Restart: a second app over the same config file.
    let loaded = h.saved();
    let Harness { dir, secrets, .. } = h;
    let restarted = Harness::on(dir, secrets, loaded);
    let after = &restarted.app.state().conn.connections;
    assert_eq!(after, &before);
    assert_eq!(
        after
            .iter()
            .filter_map(|c| c.folder.as_deref())
            .collect::<Vec<_>>(),
        vec!["Production", "Production"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saving_in_the_edit_dialog_closes_it_and_the_list_shows_the_connection() {
    let mut h = Harness::new(Vec::new());
    let form = ConnectionForm {
        name: "Fresh".into(),
        uri: "mongodb://fresh.example.com:27017".into(),
        folder: "Team".into(),
        password: MOCK_PASSWORD_MARKER.into(),
        ..ConnectionForm::default()
    };
    h.app.state_mut().conn.editing = true;
    h.app.state_mut().conn.form = form.clone();

    h.app
        .dispatch_connection(ConnectionAction::Save(Box::new(form)));

    let conn = &h.app.state().conn;
    assert!(!conn.editing, "the dialog closed");
    assert!(
        conn.form.password.is_empty(),
        "the typed password was dropped"
    );
    let rows = visible(&conn.connections, "", Sort::default());
    let groups = grouped(&rows);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].folder, Some("Team"));
    assert_eq!(groups[0].rows[0].name, "Fresh");
    assert_eq!(h.saved().len(), 1);
    assert!(!h.saved_text().contains(MOCK_PASSWORD_MARKER));
    let id = h.saved()[0].id;
    assert_eq!(h.password_of(id).as_deref(), Some(MOCK_PASSWORD_MARKER));
}
