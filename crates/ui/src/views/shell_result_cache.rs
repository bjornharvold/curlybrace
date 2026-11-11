//! What the Shell tab's Table and Tree modes draw, built once per value and kept in egui's memory
//! per session, so a repaint without a state change reuses it (the view-model rebuilds the
//! columns and flattens up to 200 documents on every call, and a Values result is copied into a
//! document each time).
//!
//! Every text is cut to a few dozen characters BEFORE any layout: a 1 MiB string is never
//! formatted or laid out per frame. The full value (itself capped) is the hover text.
//!
//! The table is keyed by the session, the run id and the identity of the result (`Arc`); the
//! tree also by the identity of its expansion (a new value on every toggle, see
//! `TreeExpansion::is_same`). An entry holds clones of those `Arc`s, so an address cannot be
//! reused by another value while it is cached.
use crate::I18n;
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_result::ShellResult;
use crate::viewmodel::shell_result_table::ResultColumns;
use crate::viewmodel::shell_result_tree::TreeExpansion;
use crate::viewmodel::{PageRow, RowDetail, TreeRow, cell_text, row_value_text};
use bson::Bson;
use eframe::egui;
use std::sync::Arc;
use uuid::Uuid;

/// Most characters a table cell or a tree value shows; the rest is cut.
pub const CELL_MAX_CHARS: usize = 80;
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

#[derive(Clone)]
struct Entry<T> {
    run_id: u64,
    result: Arc<ShellResult>,
    tree: Option<TreeExpansion>,
    data: Arc<T>,
}

impl<T> Entry<T> {
    fn fits(&self, run_id: u64, result: &Arc<ShellResult>, tree: Option<&TreeExpansion>) -> bool {
        let same_tree = match (&self.tree, tree) {
            (Some(cached), Some(wanted)) => cached.is_same(wanted),
            (None, None) => true,
            _ => false,
        };
        self.run_id == run_id && Arc::ptr_eq(&self.result, result) && same_tree
    }
}

fn table_id(session: Uuid) -> egui::Id {
    egui::Id::new(("shell_result_table_cache", session))
}

fn tree_id(session: Uuid) -> egui::Id {
    egui::Id::new(("shell_result_tree_cache", session))
}

/// The Table of `state`'s value, from the cache when it fits; `None` without a value.
pub(crate) fn table_cells(
    ctx: &egui::Context,
    session: Uuid,
    state: &ShellState,
    i18n: &I18n,
) -> Option<Arc<TableCells>> {
    cached(ctx, table_id(session), state, None, || {
        table_of(state, i18n)
    })
}

/// The Tree rows of `state`'s value, from the cache when it fits; `None` without a value.
pub(crate) fn tree_lines(
    ctx: &egui::Context,
    session: Uuid,
    state: &ShellState,
    i18n: &I18n,
) -> Option<Arc<TreeLines>> {
    let tree = Some(&state.result.tree);
    cached(ctx, tree_id(session), state, tree, || tree_of(state, i18n))
}

fn cached<T: Clone + Send + Sync + 'static>(
    ctx: &egui::Context,
    id: egui::Id,
    state: &ShellState,
    tree: Option<&TreeExpansion>,
    build: impl FnOnce() -> T,
) -> Option<Arc<T>> {
    let result = state.result.result.clone()?;
    let run_id = state.result.run_id;
    let hit = ctx
        .data(|d| d.get_temp::<Entry<T>>(id))
        .filter(|entry| entry.fits(run_id, &result, tree));
    if let Some(entry) = hit {
        return Some(entry.data);
    }
    let data = Arc::new(build());
    let entry = Entry {
        run_id,
        result,
        tree: tree.cloned(),
        data: Arc::clone(&data),
    };
    ctx.data_mut(|d| d.insert_temp(id, entry));
    Some(data)
}

/// The cached Table of `session`, as the last frame left it. For tests and diagnostics.
pub fn cached_table(ctx: &egui::Context, session: Uuid) -> Option<Arc<TableCells>> {
    ctx.data(|d| d.get_temp::<Entry<TableCells>>(table_id(session)))
        .map(|entry| entry.data)
}

/// The cached Tree rows of `session`, as the last frame left them. For tests and diagnostics.
pub fn cached_tree(ctx: &egui::Context, session: Uuid) -> Option<Arc<TreeLines>> {
    ctx.data(|d| d.get_temp::<Entry<TreeLines>>(tree_id(session)))
        .map(|entry| entry.data)
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
    let row = TreeRow {
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
    let head = prefix(text, CELL_MAX_CHARS);
    let one_line: String = head
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if head.len() == text.len() {
        return ShownText {
            shown: one_line,
            hover: None,
        };
    }
    let long = prefix(text, HOVER_MAX_CHARS);
    let hover = match long.len() == text.len() {
        true => long.to_string(),
        false => format!("{long}{}", i18n.t("shell.result.cut_hover")),
    };
    ShownText {
        shown: format!("{one_line}{}", i18n.t("shell.result.cut")),
        hover: Some(hover),
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
mod tests {
    use super::*;

    #[test]
    fn a_short_text_is_kept_whole_on_one_line() {
        let i18n = I18n::english();
        let shown = shown_text("a\nb\tc", &i18n);
        assert_eq!(shown.shown, "a b c");
        assert_eq!(shown.hover, None);
    }

    #[test]
    fn a_long_text_is_cut_by_characters_and_hovered_capped() {
        let i18n = I18n::english();
        let text = "é".repeat(1 << 20);
        let shown = shown_text(&text, &i18n);
        let cut = i18n.t("shell.result.cut");
        assert_eq!(shown.shown, format!("{}{cut}", "é".repeat(CELL_MAX_CHARS)));
        let hover = shown.hover.expect("a hover text");
        assert!(hover.starts_with(&"é".repeat(HOVER_MAX_CHARS)));
        assert!(hover.chars().count() < HOVER_MAX_CHARS + 60);
    }

    #[test]
    fn a_text_between_the_caps_hovers_whole() {
        let i18n = I18n::english();
        let text = "y".repeat(CELL_MAX_CHARS + 1);
        assert_eq!(shown_text(&text, &i18n).hover, Some(text));
    }
}
