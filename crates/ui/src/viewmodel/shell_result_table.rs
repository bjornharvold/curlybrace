//! The Table mode of a shell result: its rows (the documents, or the one object) and its columns
//! (the union of their keys, capped). Pure; the view should cache the columns per run id.
use super::cell::cell_text;
use super::shell::ShellState;
use super::shell_result::OutputMode;
use super::table::table_columns;
use crate::I18n;
use bson::Document;
use curlybrace_core::shell::ShellResult;

/// Most columns the Table shows; the rest are counted in [`ResultColumns::hidden_columns`].
pub const MAX_TABLE_COLUMNS: usize = 50;

/// The columns the Table shows, in `table_columns` order (`_id` first, then first seen).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResultColumns {
    pub shown: Vec<String>,
    /// Columns past [`MAX_TABLE_COLUMNS`], not shown.
    pub hidden_columns: usize,
}

/// `table_columns(docs)` cut to [`MAX_TABLE_COLUMNS`].
pub fn capped_columns(docs: &[Document]) -> ResultColumns {
    let all = table_columns(docs);
    let hidden_columns = all.len().saturating_sub(MAX_TABLE_COLUMNS);
    ResultColumns {
        shown: all.into_iter().take(MAX_TABLE_COLUMNS).collect(),
        hidden_columns,
    }
}

/// The text of `doc`'s `column` cell; `None` when the document has no such field.
pub fn table_cell_text(doc: &Document, column: &str, i18n: &I18n) -> Option<String> {
    doc.get(column).map(|value| cell_text(value, i18n))
}

impl ShellState {
    /// The Table's rows: the documents, an object as its one row, nothing where the Table is
    /// unavailable.
    pub fn table_docs(&self) -> &[Document] {
        match self.shown_in(OutputMode::Table) {
            Some(ShellResult::Documents { docs, .. }) => docs,
            Some(ShellResult::Object { doc, .. }) => std::slice::from_ref(doc),
            _ => &[],
        }
    }

    pub fn result_columns(&self) -> ResultColumns {
        capped_columns(self.table_docs())
    }
}
