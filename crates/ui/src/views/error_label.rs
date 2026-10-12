//! One place to draw an error: localized message plus, where allowed, a weak detail line.
use crate::I18n;
use crate::error_text::error_detail;
use curlybrace_core::error::CoreError;
use eframe::egui;

pub fn show_error(ui: &mut egui::Ui, error: &CoreError, i18n: &I18n) {
    ui.colored_label(ui.visuals().error_fg_color, i18n.error(error));
    if let Some(detail) = error_detail(error) {
        ui.weak(detail);
    }
}
