//! Screenshot tests: fixed in-memory states rendered with wgpu and compared with committed PNGs
//! in `tests/snapshots/`. No database is needed.
//!
//! Run:    `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
mod support;

use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::app::host_key::HostKeyRequest;
use curlybrace_ui::app::state::{AppState, Banner};
use curlybrace_ui::viewmodel::{
    EditorIntent, EditorState, MenuItem, ViewMode, page_key, resolve_editor,
};
use curlybrace_ui::views::connections::ConnectionsState;
use support::compare_states::{Through, compare_state};
use support::db::sample_people;
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

shot!(browser_tree_view, vec![], browsing(ViewMode::Tree));
shot!(browser_table_view, vec![], browsing(ViewMode::Table));
shot!(browser_json_view, vec![], browsing(ViewMode::Json));

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
    for (key, text) in [
        ("connections.name", "Temp"),
        ("connections.uri", "mongodb://localhost"),
    ] {
        let label = i18n.t(key);
        harness.get_by_label(&label).click();
        harness.run();
        harness.get_by_label(&label).type_text(text);
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
