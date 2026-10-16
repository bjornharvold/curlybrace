//! No secret may reach a file, an export, a log line (`Debug`) or a screen. The marker password
//! is stored through every path that handles passwords; every place a value could leak is then
//! searched for it.
mod conn_harness;

use conn_harness::{FixedPath, Harness};
use curlybrace_core::error::{CoreError, Result};
use curlybrace_core::runtime::{RuntimeConfig, TunnelHandle, TunnelOpener, TunnelRequest};
use curlybrace_core::test_support::mock_objects::{MOCK_PASSWORD_MARKER, mock_connections};
use curlybrace_ui::app::state::Banner;
use curlybrace_ui::viewmodel::ConnectionForm;
use curlybrace_ui::views::connections::{ConnectionAction, Notice, TestState};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

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
    // A banner reaches `dialog_banner` through the real path: the connections screen copies it in
    // once the dialog is open and the banner is new since it opened.
    h.app.state_mut().conn.editing = true;
    h.frame();
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

const SSH_MARKER: &str = "ssh-secret-marker-91c2";

fn form_with(name: &str, password: &str, ssh: bool) -> ConnectionForm {
    ConnectionForm {
        name: name.into(),
        username: "audit".into(),
        password: password.into(),
        ssh_enabled: ssh,
        ssh_host: if ssh {
            "bastion.invalid".into()
        } else {
            String::new()
        },
        ssh_user: if ssh { "tunnel".into() } else { String::new() },
        ssh_secret: if ssh {
            SSH_MARKER.into()
        } else {
            String::new()
        },
        ..typed_form()
    }
}

fn assert_no_secret(what: &str, text: &str, secrets: &[&str]) {
    for secret in secrets {
        assert!(!text.contains(secret), "{what} leaked a secret");
    }
}

fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Hands out a tunnel that listens nowhere and remembers the secrets it was given.
#[derive(Default)]
struct RecordingOpener {
    seen: Mutex<Vec<String>>,
}

impl RecordingOpener {
    fn secrets_seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

struct IdleTunnel;

impl TunnelHandle for IdleTunnel {
    fn local_addr(&self) -> SocketAddr {
        "127.0.0.1:50999".parse().unwrap()
    }
}

#[async_trait::async_trait]
impl TunnelOpener for RecordingOpener {
    async fn open(&self, request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>> {
        if let Some(secret) = request.secret {
            self.seen.lock().unwrap().push(secret.to_owned());
        }
        Ok(Box::new(IdleTunnel))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_open_sessions_one_through_an_ssh_tunnel_with_typed_secrets_leak_nothing_to_debug_or_disk()
 {
    let second_password = format!("{MOCK_PASSWORD_MARKER}-second");
    let secrets = [MOCK_PASSWORD_MARKER, second_password.as_str(), SSH_MARKER];
    // The tunnel is a fake that records what it was asked to open: no SSH server is involved.
    let opener = Arc::new(RecordingOpener::default());
    let config = RuntimeConfig {
        tunnel_opener: opener.clone(),
        known_hosts_path: Some(std::env::temp_dir().join("curlybrace-audit-known-hosts")),
        ..RuntimeConfig::default()
    };
    let mut h = Harness::with_config(Vec::new(), config);
    for form in [
        form_with("first", MOCK_PASSWORD_MARKER, false),
        form_with("second", &second_password, false),
        form_with("tunnelled", MOCK_PASSWORD_MARKER, true),
    ] {
        h.app
            .dispatch_connection(ConnectionAction::Save(Box::new(form)));
    }
    let ids: Vec<_> = h
        .app
        .state()
        .conn
        .connections
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(h.password_of(ids[1]).is_some(), "stored in the keychain");

    for id in &ids {
        h.app
            .dispatch(curlybrace_ui::app::reducer::Input::Connect(*id));
        h.until("connected", |a| {
            a.state().active == Some(*id)
                && a.state().status == curlybrace_ui::app::state::ConnStatus::Connected
        });
    }
    assert_eq!(
        opener.secrets_seen(),
        vec![SSH_MARKER.to_owned()],
        "the tunnel was opened with its secret (the opener is the one legitimate receiver)"
    );
    // Secrets still sitting in the open dialog are as sensitive as stored ones.
    h.app.state_mut().conn.form = form_with("draft", &second_password, true);
    assert_eq!(h.app.state().open.len(), 3);
    assert_eq!(
        h.app.state().parked.len(),
        2,
        "the other sessions are parked"
    );

    let state = h.app.state();
    assert_no_secret("Debug of the app state", &format!("{state:?}"), &secrets);
    assert_no_secret("pretty Debug", &format!("{state:#?}"), &secrets);
    assert_no_secret("Debug of parked", &format!("{:?}", state.parked), &secrets);

    let files = files_under(h.dir.path());
    assert!(!files.is_empty(), "the config file exists");
    for file in files {
        let text = String::from_utf8_lossy(&std::fs::read(&file).unwrap()).into_owned();
        assert_no_secret(&format!("{}", file.display()), &text, &secrets);
    }
}
