//! `workspace.json` holds ids, namespaces and typed query text, never a URI, a user, a host, a
//! secret or a document. A connection with a password, a user and an SSH tunnel with a typed
//! passphrase is browsed, its tabs are saved, and every byte of the file is searched.
mod conn_harness;

use conn_harness::{Harness, people};
use curlybrace_core::error::Result;
use curlybrace_core::runtime::{RuntimeConfig, TunnelHandle, TunnelOpener, TunnelRequest};
use curlybrace_core::test_support::mock_objects::MOCK_PASSWORD_MARKER;
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::ConnStatus;
use curlybrace_ui::viewmodel::{ConnectionForm, SidebarAction};
use curlybrace_ui::views::connections::ConnectionAction;
use std::net::SocketAddr;
use std::sync::Arc;

const SSH_MARKER: &str = "ssh-passphrase-marker-4b7e";
const TYPED_FILTER: &str = r#"{"_id": 1}"#;

struct IdleTunnel;

impl TunnelHandle for IdleTunnel {
    fn local_addr(&self) -> SocketAddr {
        "127.0.0.1:50998".parse().unwrap()
    }
}

/// A tunnel that listens nowhere: no SSH server is involved.
struct FakeOpener;

#[async_trait::async_trait]
impl TunnelOpener for FakeOpener {
    async fn open(&self, _request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>> {
        Ok(Box::new(IdleTunnel))
    }
}

fn tunnelled_form() -> ConnectionForm {
    ConnectionForm {
        name: "Audit".into(),
        uri: "mongodb://db.audit.example:27017".into(),
        username: "audit-user".into(),
        password: MOCK_PASSWORD_MARKER.into(),
        ssh_enabled: true,
        ssh_host: "bastion.audit.example".into(),
        ssh_user: "tunnel-user".into(),
        ssh_secret: SSH_MARKER.into(),
        ..ConnectionForm::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_file_never_holds_a_uri_or_secret() {
    // Its own folder: nothing of this test goes to the shared system temp directory.
    let hosts = tempfile::tempdir().unwrap();
    let config = RuntimeConfig {
        tunnel_opener: Arc::new(FakeOpener),
        known_hosts_path: Some(hosts.path().join("known_hosts")),
        ..RuntimeConfig::default()
    };
    let mut h = Harness::with_config(Vec::new(), config);
    let path = h.dir.path().join("workspace.json");
    let state = h.app.state_mut();
    *state = std::mem::take(state).with_workspace_path(Ok(path.clone()));
    h.app
        .dispatch_connection(ConnectionAction::Save(Box::new(tunnelled_form())));
    let id = h.app.state().conn.connections[0].id;
    assert!(
        h.password_of(id).is_some(),
        "the password is in the keychain"
    );

    h.app.dispatch(Input::Connect(id));
    h.until("connected", |a| {
        a.state().active == Some(id) && a.state().status == ConnStatus::Connected
    });
    h.app
        .dispatch(Input::Sidebar(SidebarAction::Open(people())));
    h.until("the page", |a| a.state().page.is_some());
    h.app.state_mut().find.text.filter = TYPED_FILTER.into();
    h.app.save_workspace_now();

    let bytes = std::fs::read(&path).expect("the workspace file was written");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        text.contains(&id.to_string()),
        "the connection id is saved: {text}"
    );
    assert!(text.contains("people"), "and the namespace: {text}");
    let saved: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(
        saved["tabs"][0]["find"]["filter"], TYPED_FILTER,
        "and the typed text: {text}"
    );
    for (what, secret) in [
        ("the password", MOCK_PASSWORD_MARKER),
        ("the passphrase", SSH_MARKER),
        ("the URI scheme", "mongodb://"),
        ("the server host", "db.audit.example"),
        ("the user", "audit-user"),
        ("the tunnel host", "bastion.audit.example"),
        ("the tunnel user", "tunnel-user"),
        ("a document of the page", "alice"),
    ] {
        assert!(!text.contains(secret), "the workspace file holds {what}");
    }
}
