//! No secret may reach a file, an export, a log line (`Debug`) or a screen. The marker password
//! is stored through every path that handles passwords; every place a value could leak is then
//! searched for it.
mod conn_harness;

use conn_harness::{FixedPath, Harness};
use curlybrace_core::error::CoreError;
use curlybrace_core::test_support::mock_objects::{MOCK_PASSWORD_MARKER, mock_connections};
use curlybrace_ui::app::state::Banner;
use curlybrace_ui::viewmodel::ConnectionForm;
use curlybrace_ui::views::connections::{ConnectionAction, Notice, TestState};
use std::sync::Arc;

fn assert_clean(what: &str, text: &str) {
    assert!(
        !text.contains(MOCK_PASSWORD_MARKER),
        "{what} leaked the password"
    );
}

#[test]
fn serialized_fixtures_hold_no_password() {
    let list = mock_connections();
    assert_clean("serde_json", &serde_json::to_string(&list).unwrap());
    assert_clean("pretty json", &serde_json::to_string_pretty(&list).unwrap());
}

fn typed_form() -> ConnectionForm {
    ConnectionForm {
        name: "Audit".into(),
        uri: "mongodb://localhost:27017".into(),
        username: "audit".into(),
        password: MOCK_PASSWORD_MARKER.into(),
        ssh_secret: MOCK_PASSWORD_MARKER.into(),
        ..ConnectionForm::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_output_of_a_state_full_of_secrets_is_clean() {
    let mut h = Harness::new(Vec::new());
    let form = typed_form();
    h.app.state_mut().conn.form = form.clone();
    // A pasted connection string with a password is parsed and cleared at once.
    h.app.state_mut().conn.uri_input = format!("mongodb://u:{MOCK_PASSWORD_MARKER}@host/db");
    h.app.state_mut().conn.apply_uri_input();
    h.app.state_mut().conn.form = form.clone();
    // A pasted string that has not been parsed yet is still a secret.
    h.app.state_mut().conn.uri_input = format!("mongodb://u:{MOCK_PASSWORD_MARKER}@host/db");

    h.app.dispatch_connection(ConnectionAction::BuildUri {
        form: Box::new(form.clone()),
        include_password: true,
    });
    assert!(
        h.app.state().conn.built_with_password,
        "a URI with a real password was built"
    );

    h.app
        .dispatch_connection(ConnectionAction::Test(Box::new(form)));
    h.until("test result", |a| {
        matches!(a.state().conn.test, TestState::Done { .. })
    });
    // A banner reaches `dialog_banner` through the real path: the connections screen copies it in.
    h.app.state_mut().banner = Some(Banner::Failure {
        context: "save_connection",
        error: CoreError::Secret("keychain said no".into()),
    });
    h.frame();
    assert!(
        !h.app.state().conn.dialog_banner.is_empty(),
        "the screen copied the banner into the dialog state"
    );
    assert_clean(
        "Debug of the dialog state",
        &format!("{:?}", h.app.state().conn),
    );

    // A failing test result: the typed password was sent, the error must not echo it.
    let mut failing = typed_form();
    failing.uri = "mongodb://localhost:27017".into();
    h.app.state_mut().conn.test = TestState::Done {
        request_id: 2,
        outcome: Err(CoreError::AuthFailed),
    };
    h.app.state_mut().conn.test_form = Some(failing);
    h.frame();

    let state = h.app.state();
    assert_clean(
        "Debug of the connections state",
        &format!("{:?}", state.conn),
    );
    assert_clean(
        "pretty Debug of the connections state",
        &format!("{:#?}", state.conn),
    );
    assert_clean("Debug of the app state", &format!("{state:?}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_config_file_holds_no_password_after_save_duplicate_and_import() {
    let mut h = Harness::new(Vec::new());
    h.app
        .dispatch_connection(ConnectionAction::Save(Box::new(typed_form())));
    let id = h.app.state().conn.connections[0].id;
    assert_eq!(
        h.password_of(id).as_deref(),
        Some(MOCK_PASSWORD_MARKER),
        "stored in the keychain"
    );
    assert_clean("the file after Save", &h.saved_text());

    h.app.dispatch_connection(ConnectionAction::Duplicate(id));
    assert_eq!(h.app.state().conn.connections.len(), 2);
    assert_clean("the file after Duplicate", &h.saved_text());

    let export = h.dir.path().join("export.json");
    h.app = h
        .app
        .with_file_picker(Arc::new(FixedPath(export.clone())), Arc::new(|| {}));
    h.app.dispatch_connection(ConnectionAction::StartExport);
    h.until("exported", |a| {
        matches!(a.state().conn.notice, Some(Notice::Exported { .. }))
    });
    assert_clean(
        "the export file",
        &std::fs::read_to_string(&export).unwrap(),
    );

    h.app.dispatch_connection(ConnectionAction::StartImport);
    h.until("imported", |a| {
        matches!(a.state().conn.notice, Some(Notice::Imported { .. }))
    });
    assert_eq!(h.app.state().conn.connections.len(), 4);
    assert_clean("the file after Import", &h.saved_text());
}
