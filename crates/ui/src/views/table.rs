//! Table view: one column per field name, one virtualised row per document.
use super::context_menu::row_context_menu;
use super::metrics::row_height;
use crate::I18n;
use crate::viewmodel::{DocAction, PageKey, TreeState, cell_text, menu_items, table_columns};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use mongostudio_core::store::Page;

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
    let mut chosen = None;
    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .vscroll(true);
    for _ in &columns {
        builder = builder.column(Column::initial(COLUMN_INITIAL_WIDTH).clip(true));
    }
    builder
        .header(height, |mut header| {
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
                for name in &columns {
                    row.col(|ui| {
                        let text = doc
                            .get(name)
                            .map(|b| cell_text(b, i18n))
                            .unwrap_or_default();
                        ui.add(egui::Label::new(text).truncate());
                    });
                }
                if let Some(item) =
                    row_context_menu(&row.response(), &menu_items(doc, projected), i18n)
                {
                    chosen = Some((index, item));
                }
            });
        });
    let (index, item) = chosen?;
    let doc = page.docs.get(index)?;
    let (next, action) = std::mem::take(state).menu_chosen(item, doc, key);
    *state = next;
    action
}
