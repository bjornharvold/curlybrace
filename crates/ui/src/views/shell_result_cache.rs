//! What the Shell tab's Table and Tree modes draw, built once per value and kept in egui's memory
//! per session, so a repaint without a state change reuses it (the view-model rebuilds the
//! columns and flattens up to 200 documents on every call, and a Values result is copied into a
//! document each time).
//!
//! Every text is cut to a few dozen characters BEFORE any layout: a 1 MiB string is never
//! formatted or laid out per frame. The full value (itself capped) is the hover text.
//!
//! How the entries are keyed and when they are dropped: see `shell_result_store`.
use super::shell_result_store::{
    Stored, cached, evict_stale, forget_both, known, stored, table_id, tree_id,
};
use crate::I18n;
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_result_table::ResultColumns;
use crate::viewmodel::{PageRow, RowDetail, TreeRow, cell_text, row_value_text};
use bson::Bson;
use eframe::egui;
use std::sync::Arc;
use uuid::Uuid;

/// Most characters a table cell or a tree value shows; the rest is cut.
pub const CELL_MAX_CHARS: usize = 80;
/// Most characters a tree key shows; the rest is cut.
pub const KEY_MAX_CHARS: usize = 40;
/// Most characters of a cut value its hover text shows.
pub const HOVER_MAX_CHARS: usize = 2_000;

/// One table cell or tree value as drawn: one line, cut, with the longer text to hover.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownText {
    pub shown: String,
    /// Set when `shown` was cut: the value up to [`HOVER_MAX_CHARS`].
    pub hover: Option<String>,
}

/// The Table: its columns and, per row, a cell per shown column (`None`: no such field).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableCells {
    pub columns: ResultColumns,
    pub rows: Vec<Vec<Option<ShownText>>>,
}

/// One tree row; a field's scalar value is already cut (see `value`).
#[derive(Clone, Debug, PartialEq)]
pub struct TreeLine {
    pub row: PageRow,
    /// The value text of a field row, cut; `None` for a header.
    pub value: Option<ShownText>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeLines {
    pub lines: Vec<TreeLine>,
}

impl Stored for TableCells {
    fn id(session: Uuid) -> egui::Id {
        table_id(session)
    }
}

impl Stored for TreeLines {
    fn id(session: Uuid) -> egui::Id {
        tree_id(session)
    }
}

/// The Table of `state`'s value, from the cache when it fits; `None` without a value.
pub(crate) fn table_cells(
    ctx: &egui::Context,
    session: Uuid,
    state: &ShellState,
    i18n: &I18n,
) -> Option<Arc<TableCells>> {
    cached(ctx, session, state, None, || table_of(state, i18n))
}

/// The Tree rows of `state`'s value, from the cache when it fits; `None` without a value.
pub(crate) fn tree_lines(
    ctx: &egui::Context,
    session: Uuid,
    state: &ShellState,
    i18n: &I18n,
) -> Option<Arc<TreeLines>> {
    let tree = Some(&state.result.tree);
    cached(ctx, session, state, tree, || tree_of(state, i18n))
}

/// Drops `session`'s Table and Tree rows that no longer belong to the value `state` shows; the
/// output pane calls it every frame, whichever mode is on screen.
pub(crate) fn drop_stale(ctx: &egui::Context, session: Uuid, state: &ShellState) {
    evict_stale::<TableCells, TreeLines>(ctx, session, state);
}

/// Forgets everything cached for `session`.
pub fn forget(ctx: &egui::Context, session: Uuid) {
    forget_both::<TableCells, TreeLines>(ctx, session);
}

/// Forgets every session that is not in `open` (a closed session's rows never linger).
pub fn retain_sessions(ctx: &egui::Context, open: &[Uuid]) {
    let closed: Vec<Uuid> = known(ctx)
        .iter()
        .copied()
        .filter(|session| !open.contains(session))
        .collect();
    for session in closed {
        forget(ctx, session);
    }
}

/// The cached Table of `session`, as the last frame left it. For tests and diagnostics.
pub fn cached_table(ctx: &egui::Context, session: Uuid) -> Option<Arc<TableCells>> {
    stored(ctx, session)
}

/// The cached Tree rows of `session`, as the last frame left them. For tests and diagnostics.
pub fn cached_tree(ctx: &egui::Context, session: Uuid) -> Option<Arc<TreeLines>> {
    stored(ctx, session)
}

fn table_of(state: &ShellState, i18n: &I18n) -> TableCells {
    let columns = state.result_columns();
    let rows = state
        .table_docs()
        .iter()
        .map(|doc| {
            columns
                .shown
                .iter()
                .map(|name| doc.get(name).map(|value| value_text(value, i18n)))
                .collect()
        })
        .collect();
    TableCells { columns, rows }
}

fn tree_of(state: &ShellState, i18n: &I18n) -> TreeLines {
    let lines = state
        .tree_rows()
        .into_iter()
        .map(|row| match row {
            PageRow::Header { .. } => TreeLine { row, value: None },
            PageRow::Field { doc_index, row } => field_line(doc_index, row, i18n),
        })
        .collect();
    TreeLines { lines }
}

/// A field row whose long scalar value is replaced by its cut form (the row keeps no copy).
fn field_line(doc_index: usize, row: TreeRow, i18n: &I18n) -> TreeLine {
    let value = match row.detail {
        RowDetail::Scalar => shown_text(&row.value, i18n),
        _ => shown_text(&row_value_text(&row, i18n), i18n),
    };
    let key = shown_key(&row.key, i18n);
    let row = TreeRow {
        key,
        value: String::new(),
        ..row
    };
    TreeLine {
        row: PageRow::Field { doc_index, row },
        value: Some(value),
    }
}

/// A cell's text; strings are cut from the borrowed value, never copied whole.
fn value_text(value: &Bson, i18n: &I18n) -> ShownText {
    match value {
        Bson::String(s) | Bson::Symbol(s) | Bson::JavaScriptCode(s) => shown_text(s, i18n),
        other => shown_text(&cell_text(other, i18n), i18n),
    }
}

/// `text` on one line and at most [`CELL_MAX_CHARS`] long, with the cut marked.
pub fn shown_text(text: &str, i18n: &I18n) -> ShownText {
    let (shown, cut) = one_line(text, CELL_MAX_CHARS, i18n);
    if !cut {
        return ShownText { shown, hover: None };
    }
    let long = prefix(text, HOVER_MAX_CHARS);
    let hover = match long.len() == text.len() {
        true => long.to_string(),
        false => format!("{long}{}", i18n.t("shell.result.cut_hover")),
    };
    ShownText {
        shown,
        hover: Some(hover),
    }
}

/// A key on one line and at most [`KEY_MAX_CHARS`] long, with the cut marked.
fn shown_key(key: &str, i18n: &I18n) -> String {
    one_line(key, KEY_MAX_CHARS, i18n).0
}

/// The first `max` characters of `text` with control characters as spaces, the cut marked;
/// and whether it was cut.
fn one_line(text: &str, max: usize, i18n: &I18n) -> (String, bool) {
    let head = prefix(text, max);
    let line: String = head
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    match head.len() == text.len() {
        true => (line, false),
        false => (format!("{line}{}", i18n.t("shell.result.cut")), true),
    }
}

/// The first `max` characters of `text`, found without walking the rest of it.
fn prefix(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

#[cfg(test)]
#[path = "shell_result_cache_tests.rs"]
mod tests;
