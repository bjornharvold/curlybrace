//! Shared row context menu; which entries appear is decided by `viewmodel::menu_items`.
use crate::I18n;
use crate::viewmodel::MenuItem;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use eframe::egui;

/// Shows `items` in a context menu on `response`; returns the entry clicked this frame. On a
/// `read_only` connection every entry is a write, so each is listed disabled and says why on hover.
pub fn row_context_menu(
    response: &egui::Response,
    items: &[MenuItem],
    read_only: bool,
    i18n: &I18n,
) -> Option<MenuItem> {
    let mut chosen = None;
    response.context_menu(|ui| {
        for item in items {
            let entry = ui.add_enabled(!read_only, egui::Button::new(i18n.t(item.label_key())));
            if read_only {
                entry.on_disabled_hover_text(i18n.t(READ_ONLY_REASON_KEY));
            } else if entry.clicked() {
                chosen = Some(*item);
                ui.close();
            }
        }
    });
    chosen
}
