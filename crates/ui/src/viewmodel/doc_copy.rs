//! Picking documents in the documents pane and turning the pick into a copy clipboard.
//!
//! A pick holds `_id`s (never document values) or stands for the whole query result. Both are
//! references: the clipboard it produces re-reads the documents from the server when pasted.
use super::copy_clipboard::{
    CopyClipboard, DocSelection, MENU_COPY_DOCUMENT, MENU_COPY_DOCUMENTS, MENU_COPY_QUERY_RESULT,
    PasteBlock,
};
use bson::Bson;
use curlybrace_core::copy::names::{is_excluded_collection, is_excluded_database};
use curlybrace_core::crud::id_of;
use curlybrace_core::runtime::SessionId;
use curlybrace_core::store::{Namespace, Page, Query};

/// What a Copy in the documents pane asks to copy.
#[derive(Debug, Clone, PartialEq)]
pub enum DocCopy {
    /// Hand-picked documents, by `_id`.
    Ids(Vec<Bson>),
    /// Every document of the query result shown, not just the visible page.
    Result,
}

/// The documents ticked on the page shown, or the whole query result.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DocPick {
    ids: Vec<Bson>,
    all_result: bool,
}

impl DocPick {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty() && !self.all_result
    }

    /// Whether the whole query result is picked.
    pub fn all_result(&self) -> bool {
        self.all_result
    }

    /// Whether the document with `id` is part of the pick.
    pub fn contains(&self, id: &Bson) -> bool {
        self.all_result || self.ids.contains(id)
    }

    /// The pick with `id` ticked or unticked. Unticking one row of a whole-result pick falls back
    /// to the other rows of `page`, since the result is no longer picked as a whole.
    pub fn row_toggled(self, id: &Bson, page: &Page) -> Self {
        let ids: Vec<Bson> = match self.all_result {
            true => page
                .docs
                .iter()
                .filter_map(|d| id_of(d).ok().cloned())
                .collect(),
            false => self.ids,
        };
        let next = match ids.contains(id) {
            true => ids.into_iter().filter(|i| i != id).collect(),
            false => ids.into_iter().chain([id.clone()]).collect(),
        };
        Self {
            ids: next,
            all_result: false,
        }
    }

    /// Picks the whole query result.
    pub fn all_selected(self) -> Self {
        Self {
            ids: Vec::new(),
            all_result: true,
        }
    }

    /// What Copy would copy; `None` while nothing is picked.
    pub fn copy(&self) -> Option<DocCopy> {
        match (self.all_result, self.ids.is_empty()) {
            (true, _) => Some(DocCopy::Result),
            (false, true) => None,
            (false, false) => Some(DocCopy::Ids(self.ids.clone())),
        }
    }

    /// The i18n key and count of the Copy entry for this pick; count 0 means the whole result.
    pub fn menu_label_key(&self) -> (&'static str, u64) {
        match (self.all_result, self.ids.len()) {
            (true, _) => (MENU_COPY_QUERY_RESULT, 0),
            (false, 1) => (MENU_COPY_DOCUMENT, 1),
            (false, n) => (MENU_COPY_DOCUMENTS, n as u64),
        }
    }
}

/// The Copy entry of one row's context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyEntry {
    /// Copy is clickable.
    pub enabled: bool,
    /// i18n key of the label and the count it interpolates (0: the whole query result).
    pub label: (&'static str, u64),
}

/// Copying is off for the documents of server-owned collections.
pub fn copy_blocked(ns: &Namespace) -> bool {
    is_excluded_collection(&ns.collection) || is_excluded_database(&ns.db)
}

/// The clipboard for `copy` taken from `ns` of `session`, while `query` is the one shown. The
/// whole-result clipboard keeps the filter and sort only: a projection of the shown page never
/// reaches the copy, so full documents are copied.
pub fn clipboard_for(
    copy: DocCopy,
    session: SessionId,
    ns: &Namespace,
    query: &Query,
) -> Result<CopyClipboard, PasteBlock> {
    if is_excluded_database(&ns.db) {
        return Err(PasteBlock::ExcludedDatabase);
    }
    if is_excluded_collection(&ns.collection) {
        return Err(PasteBlock::SystemSource);
    }
    let selection = match copy {
        DocCopy::Ids(ids) => DocSelection::ids(ids)?,
        DocCopy::Result => DocSelection::Query {
            filter: query.filter.clone(),
            sort: query.sort.clone(),
        },
    };
    Ok(CopyClipboard::Documents {
        session,
        ns: ns.clone(),
        selection,
    })
}

#[cfg(test)]
#[path = "doc_copy_tests.rs"]
mod tests;
