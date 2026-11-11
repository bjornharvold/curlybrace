//! The Shell tab's Tree mode: one collapsible tree per document (an object is one, an array of
//! values is one whose rows are its elements), with Expand all / Collapse all. Read-only: rows
//! are the Documents tab's (`PageRow`, `TreeRow`) drawn with its chevrons and type labels, but
//! without selection or menus. An expandable row is a button whose name says whether it is
//! expanded, and its accesskit node carries the expanded state. Rows are virtualised and come
//! from the per-session cache, so a repaint does not flatten the documents again.
use super::metrics::{fixed_row, row_height};
use super::shell_result::show_notes;
use super::shell_result_cache::{ShownText, TreeLine, tree_lines};
use super::tree::arrow;
use crate::I18n;
use crate::app::shell_actions::ShellAction;
use crate::theme::text_button;
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_result::ShellResult;
use crate::viewmodel::{PageRow, TreeRow, type_label_key};
use eframe::egui;
use uuid::Uuid;

pub(super) fn show_result_tree(
    ui: &mut egui::Ui,
    state: &ShellState,
    session: Uuid,
    i18n: &I18n,
) -> Option<ShellAction> {
    let lines = tree_lines(ui.ctx(), session, state, i18n)?;
    let result = state.result.result.as_deref()?;
    show_notes(ui, result, i18n);
    if lines.lines.is_empty() {
        ui.label(i18n.t("shell.result.empty"));
        return None;
    }
    let all = expand_buttons(ui, i18n);
    if state.result.tree.capped {
        ui.label(i18n.t("shell.result.expand_capped"));
    }
    let height = row_height(ui);
    let mut toggled = None;
    egui::ScrollArea::vertical()
        .id_salt(("shell_result_tree", session))
        .auto_shrink([false; 2])
        .show_rows(ui, height, lines.lines.len(), |ui, range| {
            for index in range {
                let Some(line) = lines.lines.get(index) else {
                    continue;
                };
                if let Some(action) = show_line(ui, line, state, result, height, i18n) {
                    toggled = Some(action);
                }
            }
        });
    all.or(toggled)
}

fn expand_buttons(ui: &mut egui::Ui, i18n: &I18n) -> Option<ShellAction> {
    ui.horizontal_wrapped(|ui| {
        let expand = text_button(ui, i18n.t("shell.result.expand_all")).clicked();
        let collapse = text_button(ui, i18n.t("shell.result.collapse_all")).clicked();
        match (expand, collapse) {
            (true, _) => Some(ShellAction::TreeExpandAll),
            (_, true) => Some(ShellAction::TreeCollapseAll),
            _ => None,
        }
    })
    .inner
}

fn show_line(
    ui: &mut egui::Ui,
    line: &TreeLine,
    state: &ShellState,
    result: &ShellResult,
    height: f32,
    i18n: &I18n,
) -> Option<ShellAction> {
    match &line.row {
        PageRow::Header { doc_index } => {
            let text = header_text(result, *doc_index, i18n);
            fixed_row(ui, height, |ui| {
                ui.label(egui::RichText::new(text).strong())
            });
            None
        }
        PageRow::Field { doc_index, row } => {
            let open = state.result.tree.is_expanded(*doc_index, &row.path);
            let value = line.value.as_ref();
            let (rect, ()) = fixed_row(ui, height, |ui| field_cells(ui, row, open, value, i18n));
            toggle(ui, rect, *doc_index, row, open, i18n)
        }
    }
}

/// The heading of one tree: "Document N", or "Values" for an array of values.
fn header_text(result: &ShellResult, doc_index: usize, i18n: &I18n) -> String {
    match result {
        ShellResult::Values { .. } => i18n.t("shell.result.values_title"),
        _ => {
            let n = (doc_index + 1).to_string();
            i18n.t_with("doc.header", &[("n", &n)])
        }
    }
}

/// An expandable row as a button over the whole row: named by its key and state, carrying the
/// expanded state for assistive technology; clicked (or Enter / Space) it toggles.
fn toggle(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    doc_index: usize,
    row: &TreeRow,
    open: bool,
    i18n: &I18n,
) -> Option<ShellAction> {
    if !row.is_expandable() {
        return None;
    }
    let id = ui.id().with(("shell_result_row", doc_index, &row.path));
    let response = ui.interact(rect, id, egui::Sense::click());
    let word = match open {
        true => i18n.t("shell.result.expanded"),
        false => i18n.t("shell.result.collapsed"),
    };
    let name = i18n.t_with(
        "shell.result.row_toggle",
        &[("key", &row.key), ("state", &word)],
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_expanded(open));
    response
        .clicked()
        .then(|| ShellAction::TreeToggle(doc_index, row.path.clone()))
}

/// Indent, chevron, key, type label (words, never colour alone) and the cut value.
fn field_cells(
    ui: &mut egui::Ui,
    row: &TreeRow,
    open: bool,
    value: Option<&ShownText>,
    i18n: &I18n,
) {
    ui.add_space(ui.spacing().indent * (row.depth + 1) as f32);
    let text = value.map(|v| v.shown.as_str()).unwrap_or_default();
    if row.is_summary() {
        ui.weak(text);
        return;
    }
    arrow(ui, row, open);
    ui.add(egui::Label::new(egui::RichText::new(&row.key).strong()).selectable(false));
    ui.add(
        egui::Label::new(egui::RichText::new(i18n.t(&type_label_key(row.type_name))).weak())
            .selectable(false),
    );
    let hover = value.and_then(|v| v.hover.as_deref());
    let label = egui::Label::new(text)
        .truncate()
        .selectable(false)
        .show_tooltip_when_elided(hover.is_none());
    let response = ui.add(label);
    if let Some(hover) = hover {
        response.on_hover_text(hover);
    }
}
