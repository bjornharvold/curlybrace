//! Shared row context menu; which entries appear is decided by `viewmodel::menu_items`.
use crate::I18n;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::{CopyEntry, MenuItem};
use eframe::egui;

/// Why Copy is disabled: the documents of a `system.*` collection are never copied.
const COPY_BLOCKED_REASON_KEY: &str = "copy.paste_blocked.system_source";

/// Shows `items` in a context menu on `response`; returns the entry clicked this frame. On a
/// `read_only` connection every write entry is listed disabled and says why on hover; Copy is not
/// a write and follows `copy` alone.
pub fn row_context_menu(
    response: &egui::Response,
    items: &[MenuItem],
    read_only: bool,
    copy: CopyEntry,
    i18n: &I18n,
) -> Option<MenuItem> {
    let mut chosen = None;
    response.context_menu(|ui| {
        for item in items {
            let clicked = match item {
                MenuItem::Copy => copy_entry(ui, copy, i18n),
                _ => write_entry(ui, *item, read_only, i18n),
            };
            if clicked {
                chosen = Some(*item);
                ui.close();
            }
        }
    });
    chosen
}

fn write_entry(ui: &mut egui::Ui, item: MenuItem, read_only: bool, i18n: &I18n) -> bool {
    let entry = ui.add_enabled(!read_only, egui::Button::new(i18n.t(item.label_key())));
    if read_only {
        entry.on_disabled_hover_text(i18n.t(READ_ONLY_REASON_KEY));
        return false;
    }
    entry.clicked()
}

fn copy_entry(ui: &mut egui::Ui, copy: CopyEntry, i18n: &I18n) -> bool {
    let (key, count) = copy.label;
    let label = i18n.t_with(key, &[("count", &count.to_string())]);
    let entry = ui.add_enabled(copy.enabled, egui::Button::new(label));
    if !copy.enabled {
        entry.on_disabled_hover_text(i18n.t(COPY_BLOCKED_REASON_KEY));
        return false;
    }
    entry.clicked()
}
