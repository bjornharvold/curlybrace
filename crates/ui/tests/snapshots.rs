//! Screenshot tests: fixed in-memory states rendered with wgpu and compared with committed PNGs
//! in `tests/snapshots/`. No database is needed.
//!
//! Run:    `MONGOSTUDIO_SNAPSHOTS=1 cargo test -p mongostudio-ui --test snapshots -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
mod support;

use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::error::CoreError;
use mongostudio_testkit::snapshots_enabled;
use mongostudio_ui::app::host_key::HostKeyRequest;
use mongostudio_ui::app::state::{AppState, Banner};
use mongostudio_ui::viewmodel::{
    EditorIntent, EditorState, MenuItem, ViewMode, page_key, resolve_editor,
};
use mongostudio_ui::views::connections::ConnectionsState;
use support::db::sample_people;
use support::shots::{Shot, browsing};

/// Declares a `#[ignore]`d, env-gated test that builds a [`Shot`] and snapshots it.
macro_rules! shot {
    ($name:ident, $connections:expr, $setup:expr) => {
        #[test]
        #[ignore = "needs a GPU: set MONGOSTUDIO_SNAPSHOTS=1"]
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
        form: mongostudio_ui::viewmodel::ConnectionForm::from_config(&first),
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

#[test]
#[ignore = "needs a GPU: set MONGOSTUDIO_SNAPSHOTS=1"]
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
