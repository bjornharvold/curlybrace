//! The shell around several open sessions, rendered offline (no GPU, no database): the named
//! host-key dialog, a collapsed root drawing no tree, and when the connection manager shows.
mod support;

use curlybrace_core::connection::{ConnectionConfig, SshAuth, SshConfig};
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::host_key::HostKeyRequest;
use curlybrace_ui::app::session::SessionView;
use curlybrace_ui::app::state::{AppState, ConnStatus, HostEndpoint, Workspace};
use curlybrace_ui::viewmodel::ViewMode;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::mpsc::Sender;
use support::shots::{browsing, build_app};

const BASTION: &str = "bastion.example.com";
const FINGERPRINT: &str = "SHA256:Zk3b0yQwXn8oVd1uR5TgPcLhJmA7sEiF9tKxYvC2WqU";

struct Rig {
    harness: Harness<'static, CurlybraceApp>,
    requests: Sender<HostKeyRequest>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn rig(connections: Vec<ConnectionConfig>, setup: impl FnOnce(&mut AppState)) -> Rig {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _enter = rt.enter();
    let (requests, rx) = std::sync::mpsc::channel();
    let (mut app, dir) = build_app(connections, Some(rx));
    setup(app.state_mut());
    let mut harness = Harness::builder()
        .with_size(eframe::egui::vec2(1000.0, 700.0))
        .build_state(|ctx, app: &mut CurlybraceApp| app.run_frame(ctx), app);
    harness.run();
    Rig {
        harness,
        requests,
        _rt: rt,
        _dir: dir,
    }
}

impl Rig {
    fn shows(&self, text: &str) -> bool {
        self.harness
            .query_all_by_label_contains(text)
            .next()
            .is_some()
    }

    fn ask_for_bastion(&mut self) {
        let (request, _waiter) = HostKeyRequest::new(BASTION, FINGERPRINT);
        self.requests.send(request).expect("queue the question");
        self.harness.run();
    }
}

fn via_bastion(name: &str) -> ConnectionConfig {
    ConnectionConfig {
        ssh: Some(SshConfig {
            host: BASTION.into(),
            port: 22,
            username: "u".into(),
            auth: SshAuth::Password,
        }),
        ..ConnectionConfig::new(name, "mongodb://db.example.com")
    }
}

/// Makes the connection at `index` the single open, active, connecting session.
fn connecting(index: usize) -> impl FnOnce(&mut AppState) {
    move |state| {
        let id = state.conn.connections[index].id;
        state.open = vec![id];
        state.active = Some(id);
        state.status = ConnStatus::Connecting;
        state.ssh_endpoint = HostEndpoint::of(&state.conn.connections[index]);
    }
}

fn for_connection(name: &str) -> String {
    I18n::english().t_with("hostkey.for_connection", &[("name", name)])
}

#[test]
fn the_host_key_dialog_names_the_connection_that_is_asking() {
    let mut r = rig(vec![via_bastion("Reporting")], connecting(0));
    r.ask_for_bastion();
    assert!(r.shows(&I18n::english().t("hostkey.title")));
    assert!(r.shows(&for_connection("Reporting")));
}

#[test]
fn the_host_key_dialog_has_no_connection_line_when_nobody_can_be_named() {
    let mut r = rig(vec![via_bastion("Reporting")], |_| {});
    r.ask_for_bastion();
    assert!(r.shows(&I18n::english().t("hostkey.title")));
    assert!(!r.shows(&for_connection("Reporting")));
    assert!(!r.shows(for_connection("").trim()));
}

#[test]
fn the_host_key_dialog_has_no_connection_line_when_two_connections_share_the_host() {
    let mut r = rig(
        vec![via_bastion("Reporting"), via_bastion("Billing")],
        |state| {
            let ids: Vec<_> = state.conn.connections.iter().map(|c| c.id).collect();
            state.open = ids.clone();
            state.active = Some(ids[0]);
            state.status = ConnStatus::Connecting;
            state.ssh_endpoint = HostEndpoint::of(&state.conn.connections[0]);
            state.parked.insert(
                ids[1],
                SessionView {
                    status: ConnStatus::Connecting,
                    ssh_endpoint: HostEndpoint::of(&state.conn.connections[1]),
                    ..SessionView::default()
                },
            );
        },
    );
    r.ask_for_bastion();
    assert!(r.shows(&I18n::english().t("hostkey.title")));
    assert!(!r.shows(&for_connection("Reporting")));
    assert!(!r.shows(&for_connection("Billing")));
}

/// A connected, browsing session named `Production` with `crm` expanded.
fn connected_rig() -> Rig {
    rig(vec![], browsing(ViewMode::Tree))
}

fn toggle_label() -> String {
    I18n::english().t_with("sidebar.toggle_named", &[("name", "Production")])
}

#[test]
fn an_expanded_root_shows_its_databases_and_collections() {
    let r = connected_rig();
    assert!(r.shows("billing") && r.shows("crm"));
    assert!(r.shows("notes"));
}

#[test]
fn clicking_the_toggle_collapses_the_root_and_draws_no_tree() {
    let mut r = connected_rig();
    r.harness.get_by_label(&toggle_label()).click();
    r.harness.run();
    assert!(!r.shows("billing"), "databases still drawn");
    assert!(!r.shows("sandbox"));
    assert!(!r.shows("notes"), "collections still drawn");
    // The header stays so the root can be opened again.
    r.harness.get_by_label(&toggle_label());
    assert!(r.shows("Production"));
}

#[test]
fn toggling_twice_shows_the_tree_again() {
    let mut r = connected_rig();
    r.harness.get_by_label(&toggle_label()).click();
    r.harness.run();
    r.harness.get_by_label(&toggle_label()).click();
    r.harness.run();
    assert!(r.shows("billing") && r.shows("notes"));
}

fn manager_marker() -> String {
    I18n::english().t("connections.new")
}

#[test]
fn with_no_sessions_the_manager_shows_and_no_root_does() {
    let r = rig(vec![], |_| {});
    assert!(r.shows(&manager_marker()));
    let toggle = I18n::english().t_with("sidebar.toggle_named", &[("name", "")]);
    assert!(!r.shows(toggle.trim()), "a root is drawn");
}

#[test]
fn a_connected_session_in_browse_shows_the_workspace_not_the_manager() {
    let r = connected_rig();
    assert!(!r.shows(&manager_marker()));
    assert!(r.shows("Production"), "the root is in the sidebar");
    assert!(r.shows("crm"));
}

#[test]
fn a_connected_session_in_connections_workspace_shows_manager_and_root_together() {
    let r = rig(vec![], |state| {
        browsing(ViewMode::Tree)(state);
        state.workspace = Workspace::Connections;
    });
    assert!(r.shows(&manager_marker()), "the manager list is missing");
    assert!(r.shows("Production"), "the root left the sidebar");
    assert!(r.shows("crm"), "the tree left the sidebar");
}

#[test]
fn an_active_connecting_session_shows_the_manager_and_its_root() {
    let r = rig(vec![via_bastion("Reporting")], connecting(0));
    assert!(r.shows(&manager_marker()));
    assert!(r.shows(&I18n::english().t("connections.connecting")));
    assert!(
        r.shows("Reporting"),
        "the connecting root is in the sidebar"
    );
}
