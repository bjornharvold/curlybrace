//! Fixed-height, fixed-width cells for the long lists (pair table, preflight rows, run overview),
//! so `ScrollArea::show_rows` can draw only the visible ones.
use crate::I18n;
use crate::viewmodel::compare::{EntryStatus, StatusIcon};
use eframe::egui;

/// Height of one table row; `show_rows` needs it fixed.
pub const ROW_HEIGHT: f32 = 24.0;
/// Most rows a list shows before it scrolls.
pub const MAX_ROWS_HEIGHT: f32 = 260.0;

/// A cell of `width`; whatever `add` draws is clipped to it.
pub fn cell<R>(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), layout, |ui| {
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

/// A single-line label cut off with an ellipsis; the hover text carries the whole text.
pub fn truncated(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(text).truncate())
        .on_hover_text(text);
}

/// Icon and label in one label, so the status never relies on colour and reads as one phrase.
pub fn status_text(icon: StatusIcon, label: &str) -> String {
    format!("{} {label}", icon.glyph())
}

/// The label of a check verdict, with the side named where it matters.
pub fn entry_status_label(status: &EntryStatus, i18n: &I18n) -> String {
    let text = match status.side_key() {
        Some(side) => i18n.t_with(status.i18n_key(), &[("side", &i18n.t(side))]),
        None => i18n.t(status.i18n_key()),
    };
    status_text(status.icon(), &text)
}

/// Column header line; the widths match the rows below it.
pub fn header(ui: &mut egui::Ui, columns: &[(f32, String)]) {
    ui.horizontal(|ui| {
        for (width, title) in columns {
            cell(ui, *width, |ui| {
                ui.strong(title);
            });
        }
    });
}
