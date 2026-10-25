//! Table view: one column per field name, one virtualised row per document.
use super::context_menu::row_context_menu;
use super::doc_select::{apply_toggle, checkbox_width, doc_checkbox, resolve_choice};
use super::metrics::row_height;
use crate::I18n;
use crate::viewmodel::{DocAction, PageKey, TreeState, cell_text, menu_items, table_columns};
use curlybrace_core::store::Page;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

const COLUMN_INITIAL_WIDTH: f32 = 140.0;

/// The table: a checkbox column, then one resizable column per field.
fn table_builder(ui: &mut egui::Ui, fields: usize, check_width: f32) -> TableBuilder<'_> {
    let builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .vscroll(true)
        .column(Column::exact(check_width));
    (0..fields).fold(builder, |builder, _| {
        builder.column(Column::initial(COLUMN_INITIAL_WIDTH).clip(true))
    })
}

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
    table_builder(ui, columns.len(), check_width)
        .header(height, |header| header_cells(header, &columns))
        .body(|body| {
            body.rows(height, page.docs.len(), |mut row| {
                let index = row.index();
                let Some(doc) = page.docs.get(index) else {
                    return;
                };
                row.col(|ui| {
                    if doc_checkbox(ui, state, page, index, i18n) {
                        toggled = Some(index);
                    }
                });
                for name in &columns {
                    row.col(|ui| cell(ui, doc, name, i18n));
                }
                let items = menu_items(doc, projected);
                let copy = state.copy_entry_for(doc);
                if let Some(item) = row_context_menu(&row.response(), &items, read_only, copy, i18n)
                {
                    chosen = Some((index, item));
                }
            });
        });
    apply_toggle(state, page, toggled);
    resolve_choice(state, page, key, chosen)
}

fn header_cells(mut header: egui_extras::TableRow<'_, '_>, columns: &[String]) {
    header.col(|_| {});
    for name in columns {
        header.col(|ui| {
            ui.strong(name);
        });
    }
}

fn cell(ui: &mut egui::Ui, doc: &bson::Document, name: &str, i18n: &I18n) {
    let text = doc
        .get(name)
        .map(|b| cell_text(b, i18n))
        .unwrap_or_default();
    ui.add(egui::Label::new(text).truncate());
}
