//! Text boxes expose their caption as their accessible name, which is what screen readers and
//! the click-driven UI tests use to find them. Fast, offline, no GPU.
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::{EditorState, FindBarState};
use curlybrace_ui::views::connections::{ConnectionsState, show_connections};
use curlybrace_ui::views::editor::show_editor;
use curlybrace_ui::views::findbar::show_findbar;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn size() -> egui::Vec2 {
    egui::vec2(900.0, 700.0)
}

#[test]
fn connection_form_boxes_are_found_by_their_caption() {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size()).build_ui_state(
        |ui, state: &mut ConnectionsState| {
            show_connections(ui, state, &I18n::english());
        },
        ConnectionsState::default(),
    );
    harness.run();
    for key in [
        "connections.name",
        "connections.uri",
        "connections.username",
        "connections.auth_source",
        "connections.password",
    ] {
        let label = i18n.t(key);
        let role = if key == "connections.password" {
            Role::PasswordInput
        } else {
            Role::TextInput
        };
        harness.get_by_role_and_label(role, &label);
    }
}

#[test]
fn find_bar_boxes_are_found_by_their_caption_and_keep_typed_text() {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size()).build_ui_state(
        |ui, state: &mut FindBarState| {
            show_findbar(ui, state, false, false, &I18n::english());
        },
        FindBarState::default(),
    );
    harness.run();
    let filter = i18n.t("findbar.filter");
    harness.get_by_label(&filter).click();
    harness.run();
    harness.get_by_label(&filter).type_text("{ \"a\": 1 }");
    harness.run();
    assert_eq!(harness.state().text.filter, "{ \"a\": 1 }");
    for key in [
        "findbar.sort",
        "findbar.projection",
        "findbar.skip",
        "findbar.limit",
    ] {
        harness.get_by_label(&i18n.t(key));
    }
}

#[test]
fn editor_text_box_is_named_by_the_dialog_title() {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size()).build_state(
        |ctx, state: &mut EditorState| {
            show_editor(ctx, state, &I18n::english());
        },
        EditorState::for_new(),
    );
    harness.run();
    let title = i18n.t("editor.title_new");
    harness.get_by_role_and_label(Role::MultilineTextInput, &title);
}
