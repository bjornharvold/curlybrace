//! Table view: one column per field name, one virtualised row per document.
use super::context_menu::row_context_menu;
use super::doc_select::{checkbox_width, pick_id, row_checkbox};
use super::metrics::row_height;
use crate::I18n;
use crate::viewmodel::{DocAction, PageKey, TreeState, cell_text, menu_items, table_columns};
use curlybrace_core::store::Page;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

const COLUMN_INITIAL_WIDTH: f32 = 140.0;

pub fn show_table(
    ui: &mut egui::Ui,
    page: &Page,
    key: &PageKey,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let columns = table_columns(&page.docs);
    if columns.is_empty() {
        return None;
    }
    let height = row_height(ui);
    let projected = state.projected;
    let read_only = state.read_only;
    let mut chosen = None;
    let mut toggled = None;
    let check_width = checkbox_width(ui);
    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .vscroll(true);
    builder = builder.column(Column::exact(check_width));
    for _ in &columns {
        builder = builder.column(Column::initial(COLUMN_INITIAL_WIDTH).clip(true));
    }
    builder
        .header(height, |mut header| {
            header.col(|_| {});
            for name in &columns {
                header.col(|ui| {
                    ui.strong(name);
                });
            }
        })
        .body(|body| {
            body.rows(height, page.docs.len(), |mut row| {
                let index = row.index();
                let Some(doc) = page.docs.get(index) else {
                    return;
                };
                row.col(|ui| {
                    let id = pick_id(doc);
                    let checked = id.is_some_and(|id| state.pick.contains(id));
                    let number = page.skip + index as u64 + 1;
                    if row_checkbox(ui, checked, id.is_some(), number, i18n) {
                        toggled = Some(index);
                    }
                });
                for name in &columns {
                    row.col(|ui| {
                        let text = doc
                            .get(name)
                            .map(|b| cell_text(b, i18n))
                            .unwrap_or_default();
                        ui.add(egui::Label::new(text).truncate());
                    });
                }
                let items = menu_items(doc, projected);
                let copy = state.copy_entry_for(doc);
                if let Some(item) = row_context_menu(&row.response(), &items, read_only, copy, i18n)
                {
                    chosen = Some((index, item));
                }
            });
        });
    if let Some(id) = toggled.and_then(|i| page.docs.get(i)).and_then(pick_id) {
        state.pick = std::mem::take(&mut state.pick).row_toggled(id, page);
    }
    let (index, item) = chosen?;
    let doc = page.docs.get(index)?;
    let (next, action) = std::mem::take(state).menu_chosen(item, doc, key);
    *state = next;
    action
}
