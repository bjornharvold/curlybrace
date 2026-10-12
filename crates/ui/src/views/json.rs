//! JSON view: virtualised monospace lines of the cached relaxed extended JSON.
use super::context_menu::row_context_menu;
use super::metrics::{fixed_row, row_height};
use crate::I18n;
use crate::viewmodel::{DocAction, MenuItem, TreeState, menu_items};
use eframe::egui;
use mongostudio_core::store::Page;

pub fn show_json(
    ui: &mut egui::Ui,
    page: &Page,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let lines = state.json_cache(page).shared_lines();
    let height = row_height(ui);
    let mut chosen: Option<(usize, MenuItem)> = None;
    egui::ScrollArea::both()
        .id_salt("json_view")
        .auto_shrink([false; 2])
        .show_rows(ui, height, lines.len(), |ui, range| {
            for line in &lines[range] {
                let (_, response) = fixed_row(ui, height, |ui| {
                    let text = egui::RichText::new(&line.text).monospace();
                    ui.add(
                        egui::Label::new(text)
                            .selectable(true)
                            .sense(egui::Sense::click()),
                    )
                });
                let items = menu_items(&page.docs[line.doc_index]);
                if let Some(item) = row_context_menu(&response, &items, i18n) {
                    chosen = Some((line.doc_index, item));
                }
            }
        });
    let (doc_index, item) = chosen?;
    let (next, action) = std::mem::take(state).menu_chosen(item, &page.docs[doc_index]);
    *state = next;
    action
}
