//! Screenshot tests: fixed in-memory states rendered with wgpu and compared with committed PNGs
//! in `tests/snapshots/`. No database is needed.
//!
//! Run:    `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
mod support;

use curlybrace_core::connection::{ConnectionConfig, SshAuth, SshConfig};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::TestReport;
use curlybrace_core::store::Topology;
use curlybrace_core::test_support::mock_objects::mock_connections;
use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::app::host_key::HostKeyRequest;
use curlybrace_ui::app::session::SessionView;
use curlybrace_ui::app::state::{AppState, Banner, ConnStatus, HostEndpoint};
use curlybrace_ui::viewmodel::{
    EditorIntent, EditorState, MenuItem, SidebarState, ViewMode, page_key, resolve_editor,
};
use curlybrace_ui::views::connections::{ConnectionsState, TestState};
use support::compare_apply_states::shots as apply;
use support::compare_export_shots as export;
use support::compare_field_shots as fields;
use support::compare_multi_states::{overview_mixed, pair_results, pairs_setup};
use support::compare_states::{Through, compare_state};
use support::db::sample_people;
use support::production_shots as production_shots_fixtures;
use support::shots::{Shot, browsing};

/// Declares a `#[ignore]`d, env-gated test that builds a [`Shot`] and snapshots it.
macro_rules! shot {
    ($name:ident, $connections:expr, $setup:expr) => {
        #[test]
        #[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
        fn $name() {
            if !snapshots_enabled(stringify!($name)) {
                return;
            }
            Shot::new($connections, $setup).snapshot(stringify!($name));
        }
    };
}

fn production() -> ConnectionConfig {
    let mut cfg = ConnectionConfig::new("Production replica", "mongodb://db.example.com:27017");
    cfg.username = Some("reporting".into());
    cfg.auth_source = Some("admin".into());
    cfg
}

fn staging() -> ConnectionConfig {
    ConnectionConfig::new("Staging", "mongodb://staging.example.com:27017")
}

fn select_first_connection(state: &mut AppState) {
    let first = state.conn.connections[0].clone();
    state.conn = ConnectionsState {
        selected: Some(first.id),
        form: curlybrace_ui::viewmodel::ConnectionForm::from_config(&first),
        ..std::mem::take(&mut state.conn)
    };
}

shot!(connection_manager_empty, vec![], |_| {});

shot!(
    connection_manager_with_selection,
    vec![production(), staging()],
    select_first_connection
);

shot!(connection_manager_list, mock_connections(), |_| {});

shot!(connection_manager_filtered, mock_connections(), |state| {
    state.conn.filter = "prod".into();
});

// The edit dialog for the TLS + SSH connection, open, with a successful test result.
shot!(connection_edit_dialog, mock_connections(), |state| {
    let secure = state.conn.connections[2].clone();
    let form = curlybrace_ui::viewmodel::ConnectionForm::from_config(&secure);
    state.conn = ConnectionsState {
        selected: Some(secure.id),
        form: form.clone(),
        editing: true,
        test: TestState::Done {
            request_id: 1,
            outcome: Ok(TestReport {
                server_version: Some("7.0.5".into()),
                tls: true,
                authenticated: true,
            }),
        },
        test_seq: 1,
        test_form: Some(form),
        ..std::mem::take(&mut state.conn)
    };
});

shot!(browser_tree_view, vec![], browsing(ViewMode::Tree));
shot!(browser_table_view, vec![], browsing(ViewMode::Table));
shot!(browser_json_view, vec![], browsing(ViewMode::Json));

/// Parks a root for a connection named `name` in the given status.
fn parked_root(state: &mut AppState, name: &str, status: ConnStatus, failure: Option<CoreError>) {
    let cfg = ConnectionConfig::new(name, "mongodb://other.example.com:27017");
    let id = cfg.id;
    state.conn.connections.push(cfg);
    state.open.push(id);
    let view = SessionView {
        status,
        failure,
        ..SessionView::default()
    };
    state.parked.insert(id, view);
}

shot!(sessions_tree_one_root, vec![], |state| {
    browsing(ViewMode::Tree)(state);
    state.topology = Some(Topology::ReplicaSet);
});

// A connected root with a database open, a root still connecting and a failed one.
shot!(sessions_tree_three_roots, vec![], |state| {
    browsing(ViewMode::Tree)(state);
    state.topology = Some(Topology::ReplicaSet);
    parked_root(state, "Staging", ConnStatus::Connecting, None);
    let refused = CoreError::Database("connection refused".into());
    parked_root(state, "Reporting", ConnStatus::Failed, Some(refused));
});

// Two connected roots: the active one with `crm` open and a parked one with `analytics` open.
shot!(sessions_tree_two_connected, vec![], |state| {
    browsing(ViewMode::Tree)(state);
    state.topology = Some(Topology::ReplicaSet);
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let sidebar = SidebarState::default()
        .with_databases(names(&["analytics", "logs"]))
        .toggled("analytics")
        .with_collections("analytics", names(&["events", "sessions"]));
    parked_root(state, "Staging", ConnStatus::Connected, None);
    let id = state.open[1];
    if let Some(view) = state.parked.get_mut(&id) {
        view.sidebar = sidebar;
        view.topology = Some(Topology::Direct);
    }
});

shot!(indexes_tab, vec![], support::indexes_shots::indexes_tab);

// The builder is taller than the standard window.
#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn index_builder() {
    if !snapshots_enabled("index_builder") {
        return;
    }
    let setup = support::indexes_shots::index_builder;
    Shot::sized((1000.0, 1000.0), vec![], setup).snapshot("index_builder");
}

shot!(
    sessions_tree_production_root,
    vec![],
    production_shots_fixtures::production_root
);

shot!(
    production_write_warning,
    vec![],
    production_shots_fixtures::write_warning
);

shot!(editor_modal, vec![], |state| {
    browsing(ViewMode::Tree)(state);
    state.editor = Some(EditorState::for_edit(&sample_people()[0]));
});

shot!(editor_modal_with_validation_error, vec![], |state| {
    browsing(ViewMode::Tree)(state);
    let mut editor = EditorState::for_edit(&sample_people()[0]);
    editor.text = "{ \"name\": ".into();
    let (editor, _) = resolve_editor(&editor, EditorIntent::Save);
    assert!(editor.error.is_some(), "the broken JSON must be rejected");
    state.editor = Some(editor);
});

shot!(delete_confirmation, vec![], |state| {
    browsing(ViewMode::Tree)(state);
    let page = state.page.clone().expect("page installed");
    let key = page_key(&page);
    let (tree, _) =
        std::mem::take(&mut state.tree).menu_chosen(MenuItem::Delete, &page.docs[1], &key);
    state.tree = tree;
});

shot!(error_banner, vec![production(), staging()], |state| {
    state.banner = Some(Banner::Failure {
        context: "connect",
        error: CoreError::AuthFailed,
    });
    // Raised by the first saved connection, so the banner names it.
    state.banner_session = Some(state.conn.connections[0].id);
});

shot!(
    compare_setup_empty,
    vec![production(), staging()],
    compare_state(Through::Empty)
);
shot!(
    compare_preflight_blocked_by_missing_index,
    vec![production(), staging()],
    compare_state(Through::Blocked)
);
#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn compare_running_with_progress() {
    if !snapshots_enabled("compare_running_with_progress") {
        return;
    }
    let setup = compare_state(Through::Running);
    let mut shot = Shot::new(vec![production(), staging()], setup);
    shot.snapshot_animated("compare_running_with_progress");
}
shot!(
    compare_results_with_rows,
    vec![production(), staging()],
    compare_state(Through::Results)
);
#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn compare_detail_pane() {
    if !snapshots_enabled("compare_detail_pane") {
        return;
    }
    // Taller than the standard window: the detail sits below the form and the result table.
    let mut shot = Shot::sized(
        (1000.0, 1000.0),
        vec![production(), staging()],
        compare_state(Through::Detail),
    );
    shot.snapshot("compare_detail_pane");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn compare_duplicates_panel() {
    if !snapshots_enabled("compare_duplicates_panel") {
        return;
    }
    // Taller than the standard window: the panel sits below the form and the run outcome.
    let mut shot = Shot::sized(
        (1000.0, 1000.0),
        vec![production(), staging()],
        compare_state(Through::Duplicates),
    );
    shot.snapshot("compare_duplicates_panel");
}

shot!(
    compare_pairs_setup,
    vec![production(), staging()],
    pairs_setup
);
shot!(
    compare_pair_results,
    vec![production(), staging()],
    pair_results
);

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn compare_overview_mixed() {
    if !snapshots_enabled("compare_overview_mixed") {
        return;
    }
    // Taller than the standard window: the form, the pair list and the panel stack up.
    let mut shot = Shot::sized(
        (1000.0, 1200.0),
        vec![production(), staging()],
        overview_mixed,
    );
    shot.snapshot("compare_overview_mixed");
}

/// An apply-flow screen in a window tall enough for the form, the lists and the panels.
macro_rules! apply_shot {
    ($name:ident, $setup:expr) => {
        #[test]
        #[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
        fn $name() {
            if !snapshots_enabled(stringify!($name)) {
                return;
            }
            let mut shot = Shot::sized((1000.0, 1200.0), vec![production(), staging()], $setup);
            shot.snapshot(stringify!($name));
        }
    };
}

apply_shot!(compare_apply_selection, apply::selection_in_results);
apply_shot!(compare_apply_pending, apply::pending_panel);
apply_shot!(compare_apply_review_with_delete, apply::review_with_delete);
apply_shot!(
    compare_apply_review_production,
    apply::review_with_delete_production
);
apply_shot!(compare_apply_finished_mixed, apply::apply_finished_mixed);
apply_shot!(compare_apply_pair_marked, apply::pair_marked_applied);
apply_shot!(
    compare_apply_pair_header_marked,
    apply::pair_header_marked_applied
);

apply_shot!(
    compare_fields_detail_checkboxes,
    fields::detail_with_field_checkboxes
);
apply_shot!(compare_fields_pending, fields::pending_with_fields);
apply_shot!(compare_fields_review, fields::review_with_fields);
apply_shot!(compare_fields_finished, fields::finished_with_field_updates);

apply_shot!(
    compare_export_pending_button,
    export::pending_panel_with_export_button
);
apply_shot!(compare_export_plan_modal, export::export_plan_modal);
apply_shot!(
    compare_export_plan_modal_mixed_sides,
    export::export_plan_modal_mixed_sides
);
apply_shot!(compare_export_report_modal, export::export_report_modal);

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn compare_export_choosing() {
    if !snapshots_enabled("compare_export_choosing") {
        return;
    }
    // The spinner keeps repainting while the save dialog is open.
    let mut shot = Shot::sized_waiting_for_dialog(
        (1000.0, 1200.0),
        vec![production(), staging()],
        export::export_choosing,
    );
    shot.snapshot_animated("compare_export_choosing");
}
apply_shot!(
    compare_export_finished_with_file_left,
    export::export_finished_with_file_left
);
apply_shot!(
    compare_export_finished_report,
    export::export_finished_report
);
apply_shot!(compare_export_copied, export::export_copied);

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn compare_export_running() {
    if !snapshots_enabled("compare_export_running") {
        return;
    }
    let mut shot = Shot::sized(
        (1000.0, 1200.0),
        vec![production(), staging()],
        export::export_running,
    );
    shot.snapshot_animated("compare_export_running");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn compare_apply_running() {
    if !snapshots_enabled("compare_apply_running") {
        return;
    }
    let mut shot = Shot::sized(
        (1000.0, 1200.0),
        vec![production(), staging()],
        apply::apply_running,
    );
    shot.snapshot_animated("compare_apply_running");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn host_key_prompt() {
    if !snapshots_enabled("host_key_prompt") {
        return;
    }
    let (request, _waiter) = HostKeyRequest::new(
        "bastion.example.com",
        "SHA256:Zk3b0yQwXn8oVd1uR5TgPcLhJmA7sEiF9tKxYvC2WqU",
    );
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(request).expect("queue the question");
    let mut shot = Shot::with_host_key_requests(vec![production()], |_| {}, rx);
    shot.snapshot("host_key_prompt");
}

// The same dialog when exactly one connecting session goes through the host: it names it.
#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn host_key_prompt_named() {
    if !snapshots_enabled("host_key_prompt_named") {
        return;
    }
    let (request, _waiter) = HostKeyRequest::new(
        "bastion.example.com",
        "SHA256:Zk3b0yQwXn8oVd1uR5TgPcLhJmA7sEiF9tKxYvC2WqU",
    );
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(request).expect("queue the question");
    let mut cfg = production();
    cfg.ssh = Some(SshConfig {
        host: "bastion.example.com".into(),
        port: 22,
        username: "tunnel".into(),
        auth: SshAuth::Password,
    });
    let mut shot = Shot::with_host_key_requests(
        vec![cfg],
        |state| {
            let id = state.conn.connections[0].id;
            state.open = vec![id];
            state.active = Some(id);
            state.status = ConnStatus::Connecting;
            state.ssh_endpoint = HostEndpoint::of(&state.conn.connections[0]);
        },
        rx,
    );
    shot.snapshot("host_key_prompt_named");
}

/// Not gated (no GPU, no database): the app the screenshots use must write only inside its temp
/// dir. Saving a connection through the UI must create the file there, never in the real config
/// directory.
#[test]
fn the_app_under_test_uses_temp_paths_not_the_developers_files() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _enter = rt.enter();
    let (app, dir) = support::shots::build_app(vec![], None);
    let mut harness = Harness::builder()
        .with_size(eframe::egui::vec2(800.0, 600.0))
        .build_state(
            |ctx, app: &mut curlybrace_ui::app::CurlybraceApp| app.run_frame(ctx),
            app,
        );
    let i18n = curlybrace_ui::I18n::english();
    harness.get_by_label(&i18n.t("connections.new")).click();
    harness.run();
    for (key, text) in [
        ("connections.name", "Temp"),
        ("connections.uri", "mongodb://localhost"),
    ] {
        let label = i18n.t(key);
        let role = eframe::egui::accesskit::Role::TextInput;
        harness.get_by_role_and_label(role, &label).click();
        harness.run();
        harness.get_by_role_and_label(role, &label).type_text(text);
        harness.run();
    }
    harness.get_by_label(&i18n.t("common.save")).click();
    harness.run();
    let saved = dir.path().join("connections.json");
    assert!(
        saved.exists(),
        "the connection list is written inside the temp dir"
    );
    let text = std::fs::read_to_string(saved).unwrap();
    assert!(text.contains("Temp"));
}
