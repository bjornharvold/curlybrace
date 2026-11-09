//! The "Fill from string" row of the edit dialog. The pasted connection string may hold a
//! password, so the box is masked by default and egui's own editor state for it (the text and
//! its undo history, which outlive `uri_input.clear()`) is wiped once the text is not needed.
use super::connections::ConnectionsState;
use crate::I18n;
use crate::theme::{FieldOptions, outlined_field_opts, text_button, tonal_button};
use eframe::egui;
use eframe::egui::widgets::text_edit::TextEditState;

/// Fixed id of the box, so its egui editor state can be addressed and wiped.
pub fn uri_input_id() -> egui::Id {
    egui::Id::new("connection_uri_input")
}

/// Drops egui's stored editor state for the box when `state` asked for it. Runs every frame the
/// app draws (also while the dialog is closed or another screen is shown).
pub fn wipe_if_pending(ctx: &egui::Context, state: &mut ConnectionsState) {
    if std::mem::take(&mut state.uri_wipe_pending) {
        ctx.data_mut(|d| d.remove::<TextEditState>(uri_input_id()));
    }
}

pub(super) fn show_from_uri(ui: &mut egui::Ui, state: &mut ConnectionsState, i18n: &I18n) {
    // The hint is the field's floating label and its accessible name.
    let options = FieldOptions {
        password: !state.uri_visible,
        id: Some(uri_input_id()),
        ..FieldOptions::default()
    };
    let label = i18n.t("connections.from_uri_hint");
    outlined_field_opts(ui, &label, &mut state.uri_input, "", options);
    ui.horizontal(|ui| {
        let toggle = if state.uri_visible {
            "connections.from_uri_hide"
        } else {
            "connections.from_uri_show"
        };
        if text_button(ui, i18n.t(toggle)).clicked() {
            state.uri_visible = !state.uri_visible;
        }
        if tonal_button(ui, i18n.t("connections.from_uri")).clicked() {
            state.apply_uri_input();
        }
    });
}
