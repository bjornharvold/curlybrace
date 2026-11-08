//! A sentence on a tone's container: the warnings and notices of the dialogs that must be read
//! in full, which a one-line chip would cut.
use crate::theme::{Tone, current_ui, shape};
use eframe::egui;

/// `text` wrapped over the full width on the container of `tone`; `strong` draws it in Medium.
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
