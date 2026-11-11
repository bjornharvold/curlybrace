//! The Shell tab's Table mode: a read-only table of the run's documents (an object is one row)
//! with a sticky header, virtualised rows and cut cells (the longer value on hover). It has the
//! Documents tab's look (card, tonal header band, zebra rows) without its selection or menus.
//! The rows come from the per-session cache, so a repaint lays out only the visible cells.
use super::metrics::row_height;
use super::shell_result::{note_text, show_notes};
use super::shell_result_cache::{ShownText, TableCells, table_cells};
use super::shell_result_store::{Scroll, first_draw_of_run};
use super::table::paint_header_cell;
use crate::I18n;
use crate::theme::{StateLayer, card, current_ui, state_layer};
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_result::ShellResult;
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use uuid::Uuid;

const COLUMN_INITIAL_WIDTH: f32 = 140.0;

pub(super) fn show_result_table(ui: &mut egui::Ui, state: &ShellState, session: Uuid, i18n: &I18n) {
    let Some(cells) = table_cells(ui.ctx(), session, state, i18n) else {
        return;
    };
    match empty_key(&cells, state) {
        Some(key) => {
            ui.label(i18n.t(key));
        }
        None => {
            let run_id = state.result.run_id;
            let reset = first_draw_of_run(ui.ctx(), Scroll::Table, session, run_id);
            show_cells(ui, &cells, session, reset);
        }
    }
}

/// What a table with nothing to draw says instead: no rows, or rows without a single field (an
/// empty object, documents that are all `{}`).
fn empty_key(cells: &TableCells, state: &ShellState) -> Option<&'static str> {
    if cells.rows.is_empty() {
        return Some("shell.result.empty");
    }
    if !cells.columns.shown.is_empty() {
        return None;
    }
    match state.result.result.as_deref() {
        Some(ShellResult::Object { .. }) => Some("shell.result.empty_object"),
        _ => Some("shell.result.no_fields"),
    }
}

/// The Table's part of the output heading row: its notes as one label, on that row so they take
/// no height of their own.
pub(super) fn show_table_bar(ui: &mut egui::Ui, state: &ShellState, session: Uuid, i18n: &I18n) {
    let Some(cells) = table_cells(ui.ctx(), session, state, i18n) else {
        return;
    };
    let left_out = state
        .result
        .result
        .as_deref()
        .and_then(|result| note_text(result, i18n));
    let parts = match empty_key(&cells, state) {
        Some(_) => Vec::from_iter(left_out),
        None => notes(&cells, left_out, i18n),
    };
    show_notes(ui, &parts, i18n);
}

/// "Rows: N", then what the value leaves out, then the columns not shown.
fn notes(cells: &TableCells, left_out: Option<String>, i18n: &I18n) -> Vec<String> {
    let count = cells.rows.len().to_string();
    let rows = i18n.t_with("shell.result.row_count", &[("count", &count)]);
    let hidden = cells.columns.hidden_columns;
    let columns = (hidden > 0).then(|| {
        let count = hidden.to_string();
        i18n.t_with("shell.result.more_columns", &[("count", &count)])
    });
    std::iter::once(rows)
        .chain(left_out)
        .chain(columns)
        .collect()
}

/// `reset`: a new run's value, so both scroll offsets go back to the start (the ids stay, so the
/// column widths do too).
fn show_cells(ui: &mut egui::Ui, cells: &TableCells, session: Uuid, reset: bool) {
    let height = row_height(ui);
    card(ui, |ui| {
        let s = current_ui(ui);
        // As in the Documents table: a hovered row gets the hover layer over the card's surface.
        ui.visuals_mut().widgets.hovered.bg_fill =
            state_layer(s.surface_container_low, s.on_surface, StateLayer::Hover);
        let mut horizontal = egui::ScrollArea::horizontal()
            .id_salt(("shell_result_table_h", session))
            .auto_shrink([false, true]);
        if reset {
            horizontal = horizontal.horizontal_scroll_offset(0.0);
        }
        horizontal.show(ui, |ui| {
            let viewport = ui.available_rect_before_wrap().intersect(ui.clip_rect());
            let columns = &cells.columns.shown;
            let mut table = TableBuilder::new(ui);
            if reset {
                table = table.vertical_scroll_offset(0.0);
            }
            table
                .id_salt(("shell_result_table", session))
                .striped(true)
                .resizable(true)
                .vscroll(true)
                .auto_shrink([false, false])
                // Fill the pane, however tall (the default stops at 800 points), and give way
                // in a short one rather than push the rows below the window.
                .min_scrolled_height(0.0)
                .max_scroll_height(f32::INFINITY)
                .columns(
                    Column::initial(COLUMN_INITIAL_WIDTH).clip(true),
                    columns.len(),
                )
                .header(height, |mut header| {
                    for name in columns {
                        header.col(|ui| {
                            paint_header_cell(ui, s, viewport);
                            ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate());
                        });
                    }
                })
                .body(|body| {
                    body.rows(height, cells.rows.len(), |mut row| {
                        let Some(values) = cells.rows.get(row.index()) else {
                            return;
                        };
                        for value in values {
                            row.col(|ui| show_cell(ui, value.as_ref()));
                        }
                    });
                });
        });
    });
}

/// A cell: nothing for a missing field; a cut value hovers its longer text.
fn show_cell(ui: &mut egui::Ui, value: Option<&ShownText>) {
    let Some(value) = value else {
        return;
    };
    let label = egui::Label::new(value.shown.as_str())
        .truncate()
        .show_tooltip_when_elided(value.hover.is_none());
    let response = ui.add(label);
    if let Some(hover) = &value.hover {
        response.on_hover_text(hover.as_str());
    }
}
