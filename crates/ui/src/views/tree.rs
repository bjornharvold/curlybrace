//! Tree view: virtualised rows over the flattened page (see `viewmodel::tree_state`).
use super::context_menu::row_context_menu;
use super::metrics::{fixed_row, row_height};
use crate::I18n;
use crate::viewmodel::{
    DocAction, MenuItem, PageKey, PageRow, TreeRow, TreeState, menu_items, row_value_text,
    type_label_key,
};
use curlybrace_core::store::Page;
use eframe::egui;

const ARROW_OPEN: &str = "▾";
const ARROW_CLOSED: &str = "▸";
const ARROW_NONE: &str = " ";

/// What happened on one row this frame.
#[derive(Default)]
struct RowEvent {
    clicked: bool,
    chosen: Option<MenuItem>,
}

pub fn show_tree(
    ui: &mut egui::Ui,
    page: &Page,
    key: &PageKey,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let rows = state.page_rows(page, key);
    let height = row_height(ui);
    let mut event: Option<(usize, RowEvent)> = None;
    egui::ScrollArea::vertical()
        .id_salt("tree_view")
        .auto_shrink([false; 2])
        .show_rows(ui, height, rows.len(), |ui, range| {
            for index in range {
                let Some(row) = rows.get(index) else { continue };
                if page.docs.get(row.doc_index()).is_none() {
                    continue;
                }
                let got = show_row(ui, row, page, state, height, i18n);
                if got.clicked || got.chosen.is_some() {
                    event = Some((index, got));
                }
            }
        });
    let (index, got) = event?;
    apply_event(state, rows.get(index)?, page, key, got)
}

/// Feeds one row event through the view state; returns the emitted action, if any.
fn apply_event(
    state: &mut TreeState,
    row: &PageRow,
    page: &Page,
    key: &PageKey,
    event: RowEvent,
) -> Option<DocAction> {
    let doc_index = row.doc_index();
    if let (true, PageRow::Field { row: field, .. }) = (event.clicked, row) {
        *state = std::mem::take(state).row_clicked(doc_index, field);
    }
    let item = event.chosen?;
    let doc = page.docs.get(doc_index)?;
    let (next, action) = std::mem::take(state).menu_chosen(item, doc, key);
    *state = next;
    action
}

fn show_row(
    ui: &mut egui::Ui,
    row: &PageRow,
    page: &Page,
    state: &TreeState,
    height: f32,
    i18n: &I18n,
) -> RowEvent {
    let id = ui.id().with(row_id(row));
    let (rect, ()) = fixed_row(ui, height, |ui| match row {
        PageRow::Header { doc_index } => {
            let n = page.skip + *doc_index as u64 + 1;
            let text = i18n.t_with("doc.header", &[("n", &n.to_string())]);
            ui.label(egui::RichText::new(text).strong());
        }
        PageRow::Field { doc_index, row } => {
            let open = state.is_expanded(*doc_index, &row.path);
            field_cells(ui, row, open, i18n);
        }
    });
    let response = ui.interact(rect, id, egui::Sense::click());
    let items = page
        .docs
        .get(row.doc_index())
        .map(|doc| menu_items(doc, state.projected))
        .unwrap_or_default();
    let chosen = row_context_menu(&response, &items, state.read_only, i18n);
    RowEvent {
        clicked: response.clicked(),
        chosen,
    }
}

fn row_id(row: &PageRow) -> (usize, Vec<String>) {
    match row {
        PageRow::Header { doc_index } => (*doc_index, Vec::new()),
        PageRow::Field { doc_index, row } => (*doc_index, row.path.clone()),
    }
}

fn field_cells(ui: &mut egui::Ui, row: &TreeRow, open: bool, i18n: &I18n) {
    ui.add_space(ui.spacing().indent * (row.depth + 1) as f32);
    if row.is_summary() {
        ui.weak(row_value_text(row, i18n));
        return;
    }
    ui.label(arrow(row, open));
    ui.label(egui::RichText::new(&row.key).strong());
    ui.weak(i18n.t(&type_label_key(row.type_name)));
    ui.add(egui::Label::new(row_value_text(row, i18n)).truncate());
}

fn arrow(row: &TreeRow, open: bool) -> &'static str {
    match (row.is_expandable(), open) {
        (false, _) => ARROW_NONE,
        (true, true) => ARROW_OPEN,
        (true, false) => ARROW_CLOSED,
    }
}
