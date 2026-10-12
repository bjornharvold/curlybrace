//! Shared row context menu; which entries appear is decided by `viewmodel::menu_items`.
use crate::I18n;
use crate::viewmodel::MenuItem;
use eframe::egui;

/// Shows `items` in a context menu on `response`; returns the entry clicked this frame.
pub fn row_context_menu(
    response: &egui::Response,
    items: &[MenuItem],
    i18n: &I18n,
) -> Option<MenuItem> {
    let mut chosen = None;
    response.context_menu(|ui| {
        for item in items {
            if ui.button(i18n.t(item.label_key())).clicked() {
                chosen = Some(*item);
                ui.close();
            }
        }
    });
    chosen
}
