//! Every screen that leads to a write names the connection it will write to: the documents
//! header, the delete confirmation and the editor. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::{EditorState, MenuItem, ViewMode, page_key};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use support::db::sample_people;
use support::shots::{browsing, build_app};

fn harness(
    setup: impl FnOnce(&mut AppState),
) -> (
    Harness<'static, CurlybraceApp>,
    tokio::runtime::Runtime,
    tempfile::TempDir,
) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    setup(app.state_mut());
    let mut harness = Harness::builder()
        .with_size(eframe::egui::vec2(1000.0, 700.0))
        .build_state(|ctx, app: &mut CurlybraceApp| app.run_frame(ctx), app);
    harness.run();
    (harness, rt, dir)
}

fn named(name: &str) -> impl FnOnce(&mut AppState) {
    let name = name.to_string();
    move |state| {
        browsing(ViewMode::Tree)(state);
        state.conn.connections[0].name = name;
    }
}

fn shows(harness: &Harness<'static, CurlybraceApp>, text: &str) -> bool {
    harness.query_all_by_label_contains(text).next().is_some()
}

fn long_name() -> String {
    format!("Prod {}", "ключ-✓-".repeat(60))
}

#[test]
fn the_documents_header_names_the_active_connection() {
    let (h, _rt, _dir) = harness(named("Production"));
    let line = I18n::english().t_with("doc.connection", &[("name", "Production")]);
    assert!(shows(&h, &line), "missing {line:?}");
}

#[test]
fn the_documents_header_survives_a_very_long_name() {
    let name = long_name();
    let (h, _rt, _dir) = harness(named(&name));
    let line = I18n::english().t_with("doc.connection", &[("name", &name)]);
    assert!(shows(&h, &line));
}

fn asking_to_delete(name: &str) -> impl FnOnce(&mut AppState) {
    let setup = named(name);
    move |state| {
        setup(state);
        let page = state.page.clone().expect("page installed");
        let (tree, _) = std::mem::take(&mut state.tree).menu_chosen(
            MenuItem::Delete,
            &page.docs[1],
            &page_key(&page),
        );
        state.tree = tree;
    }
}

#[test]
fn the_delete_confirmation_names_the_connection() {
    let (h, _rt, _dir) = harness(asking_to_delete("Production"));
    let id = curlybrace_ui::viewmodel::id_preview(&sample_people()[1].get("_id").unwrap().clone());
    let ask = I18n::english().t_with(
        "doc.confirm_delete",
        &[("id", &id), ("connection", "Production")],
    );
    assert!(ask.contains("Production"));
    assert!(shows(&h, &ask), "missing {ask:?}");
}

#[test]
fn the_delete_confirmation_survives_a_very_long_name() {
    let name = long_name();
    let (h, _rt, _dir) = harness(asking_to_delete(&name));
    let ask = I18n::english().t_with("doc.confirm_delete", &[("id", "1"), ("connection", &name)]);
    assert!(ask.contains(&name));
    assert!(shows(&h, &name), "the long name is on screen");
}

fn editing(name: &str) -> impl FnOnce(&mut AppState) {
    let setup = named(name);
    move |state| {
        setup(state);
        state.editor = Some(EditorState::for_edit(&sample_people()[0]));
    }
}

#[test]
fn the_editor_names_the_connection_it_will_save_to() {
    let (h, _rt, _dir) = harness(editing("Production"));
    let line = I18n::english().t_with("editor.writing_to", &[("connection", "Production")]);
    assert!(shows(&h, &line), "missing {line:?}");
}

#[test]
fn the_editor_survives_a_very_long_name() {
    let name = long_name();
    let (h, _rt, _dir) = harness(editing(&name));
    let line = I18n::english().t_with("editor.writing_to", &[("connection", &name)]);
    assert!(shows(&h, &line));
}
