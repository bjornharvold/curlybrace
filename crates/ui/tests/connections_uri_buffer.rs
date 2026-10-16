//! The "Fill from string" box holds a pasted connection string that may carry a password: it is
//! masked on screen and egui's own editor state (text cursor and undo history) is wiped as soon as
//! the pasted text is no longer needed.
use curlybrace_ui::I18n;
use curlybrace_ui::views::connections::{ConnectionsState, show_connections};
use curlybrace_ui::views::connections_from_uri::uri_input_id;
use eframe::egui;
use eframe::egui::accesskit::Role;
use eframe::egui::text::CCursorRange;
use eframe::egui::widgets::text_edit::TextEditState;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const SECRET: &str = "mongodb://bob:hunter2-marker@db.example.com/app";

fn harness() -> Harness<'static, ConnectionsState> {
    let state = ConnectionsState {
        editing: true,
        ..ConnectionsState::default()
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 900.0))
        .build_ui_state(
            |ui, state: &mut ConnectionsState| {
                show_connections(ui, state, &I18n::english());
            },
            state,
        );
    harness.run();
    harness
}

/// True when egui still remembers something typed into the box (undo history).
fn remembers_text(ctx: &egui::Context) -> bool {
    TextEditState::load(ctx, uri_input_id()).is_some_and(|state| {
        let mut undoer = state.undoer();
        undoer
            .undo(&(CCursorRange::default(), String::new()))
            .is_some()
    })
}

/// True when egui keeps no editor state at all for the box (it is not drawn, so none is rebuilt).
fn editor_state_gone(ctx: &egui::Context) -> bool {
    TextEditState::load(ctx, uri_input_id()).is_none()
}

fn field_label() -> String {
    I18n::english().t("connections.from_uri_hint")
}

/// Pastes the secret into the box and lets egui record its undo history.
fn paste(harness: &mut Harness<'static, ConnectionsState>) {
    harness
        .get_by_role_and_label(Role::PasswordInput, &field_label())
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::PasswordInput, &field_label())
        .type_text(SECRET);
    // kittest advances egui's clock by a quarter second per step, so five steps pass the undoer's
    // one second threshold and the paste is recorded as an undo point.
    for _ in 0..5 {
        harness.step();
    }
    assert_eq!(harness.state().uri_input, SECRET);
    assert!(remembers_text(&harness.ctx), "precondition: undo history");
}

fn settle(harness: &mut Harness<'static, ConnectionsState>) {
    harness.run();
    harness.run();
}

#[test]
fn apply_wipes_the_editor_state() {
    let mut harness = harness();
    paste(&mut harness);
    let apply = I18n::english().t("connections.from_uri");
    harness.get_by_role_and_label(Role::Button, &apply).click();
    settle(&mut harness);
    assert!(harness.state().uri_input.is_empty());
    assert!(!remembers_text(&harness.ctx));
}

#[test]
fn cancel_wipes_the_editor_state() {
    let mut harness = harness();
    paste(&mut harness);
    let cancel = I18n::english().t("common.cancel");
    harness.get_by_role_and_label(Role::Button, &cancel).click();
    settle(&mut harness);
    assert!(!harness.state().editing);
    assert!(editor_state_gone(&harness.ctx));
}

#[test]
fn reset_tools_wipes_the_editor_state_even_with_the_dialog_closed() {
    let mut harness = harness();
    paste(&mut harness);
    harness.state_mut().editing = false;
    harness.state_mut().reset_tools();
    settle(&mut harness);
    assert!(editor_state_gone(&harness.ctx));
}

#[test]
fn the_box_is_masked_until_shown_and_masked_again_after_reopening() {
    let i18n = I18n::english();
    let mut harness = harness();
    harness.get_by_role_and_label(Role::PasswordInput, &field_label());
    assert!(!harness.state().uri_visible);
    harness
        .get_by_role_and_label(Role::Button, &i18n.t("connections.from_uri_show"))
        .click();
    settle(&mut harness);
    harness.get_by_role_and_label(Role::TextInput, &field_label());
    harness.get_by_role_and_label(Role::Button, &i18n.t("connections.from_uri_hide"));
    harness.state_mut().close_edit();
    harness.state_mut().editing = true;
    settle(&mut harness);
    harness.get_by_role_and_label(Role::PasswordInput, &field_label());
}

#[test]
fn the_masked_box_does_not_expose_the_pasted_text_to_accessibility() {
    let mut harness = harness();
    paste(&mut harness);
    let label = field_label();
    let node = harness.get_by_role_and_label(Role::PasswordInput, &label);
    let value = node.accesskit_node().value().unwrap_or_default();
    assert!(!value.contains("hunter2"), "masked value leaked: {value}");
    assert!(!value.contains("mongodb"), "masked value leaked: {value}");
}
