//! Tree view: virtualised rows over the flattened page (see `viewmodel::tree_state`).
use super::context_menu::row_context_menu;
use super::doc_select::{checkbox_width, pick_id, row_checkbox};
use super::metrics::{fixed_row, row_height};
use crate::I18n;
use crate::theme::icons;
use crate::viewmodel::copy_clipboard::MENU_COPY_DOCUMENT;
use crate::viewmodel::{
    CopyEntry, DocAction, MenuItem, PageKey, PageRow, TreeRow, TreeState, menu_items,
    row_value_text, type_label_key,
};
use curlybrace_core::store::Page;
use eframe::egui;

/// What happened on one row this frame.
#[derive(Default)]
struct RowEvent {
    clicked: bool,
    /// The document's pick checkbox was toggled.
    toggled: bool,
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
                if got.clicked || got.toggled || got.chosen.is_some() {
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
    if let (true, Some(id)) = (event.toggled, page.docs.get(doc_index).and_then(pick_id)) {
        state.pick = std::mem::take(&mut state.pick).row_toggled(id, page);
    }
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
    let (rect, toggled) = fixed_row(ui, height, |ui| match row {
        PageRow::Header { doc_index } => header_cells(ui, *doc_index, page, state, i18n),
        PageRow::Field { doc_index, row } => {
            let open = state.is_expanded(*doc_index, &row.path);
            field_cells(ui, row, open, i18n);
            false
        }
    });
    // The row senses clicks over the content, so a header's checkbox is left out of it.
    let sensed = match row {
        PageRow::Header { .. } => rect.with_min_x(rect.min.x + checkbox_width(ui)),
        PageRow::Field { .. } => rect,
    };
    let response = ui.interact(sensed, id, egui::Sense::click());
    let items = page
        .docs
        .get(row.doc_index())
        .map(|doc| menu_items(doc, state.projected))
        .unwrap_or_default();
    let copy = page
        .docs
        .get(row.doc_index())
        .map(|doc| state.copy_entry_for(doc))
        .unwrap_or(CopyEntry {
            enabled: false,
            label: (MENU_COPY_DOCUMENT, 1),
        });
    let chosen = row_context_menu(&response, &items, state.read_only, copy, i18n);
    RowEvent {
        clicked: response.clicked(),
        toggled,
        chosen,
    }
}

/// The document heading with the checkbox that picks the document for Copy before it; true when
/// toggled. The heading stays a plain label so a right-click on it opens the row menu.
fn header_cells(
    ui: &mut egui::Ui,
    doc_index: usize,
    page: &Page,
    state: &TreeState,
    i18n: &I18n,
) -> bool {
    let n = page.skip + doc_index as u64 + 1;
    let id = page.docs.get(doc_index).and_then(pick_id);
    let checked = id.is_some_and(|id| state.pick.contains(id));
    let toggled = row_checkbox(ui, checked, id.is_some(), n, i18n);
    let text = i18n.t_with("doc.header", &[("n", &n.to_string())]);
    ui.label(egui::RichText::new(text).strong());
    toggled
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
    arrow(ui, row, open);
    ui.label(egui::RichText::new(&row.key).strong());
    ui.weak(i18n.t(&type_label_key(row.type_name)));
    ui.add(egui::Label::new(row_value_text(row, i18n)).truncate());
}

/// The disclosure chevron of an expandable row. Every row reserves the same slot (the wider of
/// the two chevrons), so keys line up whether or not a row can be expanded or is open.
fn arrow(ui: &mut egui::Ui, row: &TreeRow, open: bool) {
    let (rect, _) = ui.allocate_exact_size(chevron_slot(ui), egui::Sense::hover());
    if row.is_expandable() && ui.is_rect_visible(rect) {
        let color = ui.visuals().text_color();
        ui.painter()
            .galley(rect.min, chevron_galley(ui, open), color);
    }
}

/// The size of the chevron slot (the wider of the two chevrons). It depends only on the fonts, so
/// it is measured once per pass and shared by every row of that pass, not laid out per row.
fn chevron_slot(ui: &egui::Ui) -> egui::Vec2 {
    let id = egui::Id::new("tree_chevron_slot");
    let pass = ui.ctx().cumulative_pass_nr();
    let cached = ui.data(|d| d.get_temp::<(u64, egui::Vec2)>(id));
    if let Some((_, slot)) = cached.filter(|(cached, _)| *cached == pass) {
        return slot;
    }
    let slot = [false, true]
        .map(|state| chevron_galley(ui, state).size())
        .into_iter()
        .fold(egui::Vec2::ZERO, |widest, size| widest.max(size));
    ui.data_mut(|d| d.insert_temp(id, (pass, slot)));
    slot
}

fn chevron_galley(ui: &egui::Ui, open: bool) -> std::sync::Arc<egui::Galley> {
    egui::WidgetText::from(icons::inline_icon(ui, icons::chevron(open))).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;
    use crate::viewmodel::page_key;
    use bson::doc;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    struct Pane {
        page: Page,
        state: TreeState,
    }

    fn open() -> Harness<'static, Pane> {
        let page = Page {
            docs: vec![doc! { "name": "Ada", "address": { "city": "London" } }],
            skip: 0,
            has_more: false,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 300.0))
            .build_ui_state(
                |ui, pane: &mut Pane| {
                    let key = page_key(&pane.page);
                    show_tree(ui, &pane.page, &key, &mut pane.state, &I18n::english());
                },
                Pane {
                    page,
                    state: TreeState::default(),
                },
            );
        theme::install(&harness.ctx);
        harness.run_steps(4);
        harness
    }

    #[test]
    fn keys_line_up_between_leaf_rows_and_expandable_rows() {
        let harness = open();
        let leaf = harness.get_by_label("name").rect().min.x;
        let expandable = harness.get_by_label("address").rect().min.x;
        assert!(
            (leaf - expandable).abs() < 0.01,
            "leaf key at {leaf}, expandable key at {expandable}"
        );
    }
}
