//! The "Fill from string" row of the edit dialog. The pasted connection string may hold a
//! password, so the box is masked by default and egui's own editor state for it (the text and
//! its undo history, which outlive `uri_input.clear()`) is wiped once the text is not needed.
use super::connections::ConnectionsState;
use crate::I18n;
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
    ui.horizontal(|ui| {
        let caption = ui.label(i18n.t("connections.from_uri_hint"));
        ui.add(
            egui::TextEdit::singleline(&mut state.uri_input)
                .id(uri_input_id())
                .password(!state.uri_visible),
        )
        .labelled_by(caption.id);
        let toggle = if state.uri_visible {
            "connections.from_uri_hide"
        } else {
            "connections.from_uri_show"
        };
        if ui.button(i18n.t(toggle)).clicked() {
            state.uri_visible = !state.uri_visible;
        }
        if ui.button(i18n.t("connections.from_uri")).clicked() {
            state.apply_uri_input();
        }
    });
}
