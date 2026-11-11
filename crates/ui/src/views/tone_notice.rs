//! A sentence on a tone's container: the warnings and notices of the dialogs that must be read
//! in full, which a one-line chip would cut.
use crate::theme::{Tone, current_ui, shape};
use eframe::egui;

/// `text` wrapped over the full width on the container of `tone`, in the tone's `on_container`
/// color. `strong` sets egui's strong flag, which only recolors text and so changes nothing here
/// (the color is explicit); it does not switch to the Medium weight.
pub fn tone_notice(ui: &mut egui::Ui, tone: Tone, text: &str, strong: bool) {
    let colors = current_ui(ui).tone(tone);
    egui::Frame::new()
        .fill(colors.container)
        .corner_radius(shape::radius(shape::MEDIUM))
        .inner_margin(egui::Margin::symmetric(
            shape::SPACE_3 as i8,
            shape::SPACE_2 as i8,
        ))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let text = egui::RichText::new(text).color(colors.on_container);
            ui.label(if strong { text.strong() } else { text });
        });
}

/// `text` on the container of `tone`, only as wide as it needs (it still wraps at the end of
/// the row): a status that shares a row with buttons, no taller than they are.
pub fn tone_status(ui: &mut egui::Ui, tone: Tone, text: &str) {
    let colors = current_ui(ui).tone(tone);
    egui::Frame::new()
        .fill(colors.container)
        .corner_radius(shape::radius(shape::MEDIUM))
        .inner_margin(egui::Margin::symmetric(
            shape::SPACE_3 as i8,
            shape::SPACE_1 as i8,
        ))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(colors.on_container));
        });
}
