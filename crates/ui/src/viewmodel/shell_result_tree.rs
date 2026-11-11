//! The Tree mode of a shell result: which paths are expanded and the rows that follow from it.
//! Rows are the Documents tab's (`flatten_page`, `PageRow`, `TreeRow`), so the view renders them
//! with `row_value_text` and `type_label_key`. Pure; every updater returns a new value.
//!
//! The tree's documents: the result's documents, an object as one document, and an array of other
//! values as one document keyed `"0"`, `"1"`, ... (its elements as top-level rows).
use super::shell::ShellState;
use super::shell_result::{OutputMode, ResultView};
use super::tree::MAX_ROWS_PER_LEVEL;
use super::tree_state::{PageRow, flatten_page};
use bson::{Bson, Document};
use curlybrace_core::shell::ShellResult;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

/// Most paths "expand all" opens across every document of a result.
pub const MAX_EXPANDED_PATHS: usize = 5_000;
/// Most rows "expand all" reveals below the top level (each opened path reveals up to
/// `MAX_ROWS_PER_LEVEL`); with the paths cap it bounds the rows of a fully expanded tree.
pub const MAX_EXPAND_ALL_ROWS: usize = 50_000;

type Expanded = HashMap<usize, HashSet<Vec<String>>>;

/// The expanded paths of each document, by document index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeExpansion {
    open: Arc<Expanded>,
    /// The last "expand all" stopped at a cap, so some containers stay closed.
    pub capped: bool,
}

impl TreeExpansion {
    pub fn is_expanded(&self, doc_index: usize, path: &[String]) -> bool {
        self.open
            .get(&doc_index)
            .is_some_and(|paths| paths.contains(path))
    }

    /// Number of expanded paths across every document.
    pub fn len(&self) -> usize {
        self.open.values().map(HashSet::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The same expansion value (not just equal paths): every change makes a new one, so a view
    /// cache keyed on it holds one clone and compares in O(1).
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.open, &other.open) && self.capped == other.capped
    }

    fn toggled(&self, doc_index: usize, path: Vec<String>) -> Self {
        let mut open = (*self.open).clone();
        let paths = open.entry(doc_index).or_default();
        if !paths.remove(&path) {
            paths.insert(path);
        }
        Self {
            open: Arc::new(open),
            capped: self.capped,
        }
    }
}

impl ShellState {
    /// The documents the Tree shows; empty where the Tree is unavailable.
    pub fn tree_docs(&self) -> Cow<'_, [Document]> {
        match self.shown_in(OutputMode::Tree) {
            Some(ShellResult::Documents { docs, .. }) => Cow::Borrowed(docs.as_slice()),
            Some(ShellResult::Object { doc, .. }) => Cow::Borrowed(std::slice::from_ref(doc)),
            Some(ShellResult::Values { values, .. }) => Cow::Owned(vec![indexed(values)]),
            _ => Cow::Borrowed(&[]),
        }
    }

    /// Every document's header and its rows, with the expanded paths open.
    pub fn tree_rows(&self) -> Vec<PageRow> {
        flatten_page(&self.tree_docs(), &self.result.tree.open)
    }

    /// Opens or closes `path` of document `doc_index`; ignored without a tree or such a document.
    pub fn tree_toggled(&self, doc_index: usize, path: Vec<String>) -> Self {
        if doc_index >= self.tree_doc_count() {
            return self.clone();
        }
        self.with_tree(self.result.tree.toggled(doc_index, path))
    }

    /// Expands every container (up to the caps, breadth first, so the top levels open first) or
    /// collapses everything. Ignored without a tree.
    pub fn tree_all_set(&self, expanded: bool) -> Self {
        if self.tree_doc_count() == 0 {
            return self.clone();
        }
        let tree = match expanded {
            true => every_expandable(&self.tree_docs()),
            false => TreeExpansion::default(),
        };
        self.with_tree(tree)
    }

    fn tree_doc_count(&self) -> usize {
        match self.shown_in(OutputMode::Tree) {
            Some(ShellResult::Documents { docs, .. }) => docs.len(),
            Some(ShellResult::Object { .. } | ShellResult::Values { .. }) => 1,
            _ => 0,
        }
    }

    fn with_tree(&self, tree: TreeExpansion) -> Self {
        Self {
            result: ResultView {
                tree,
                ..self.result.clone()
            },
            ..self.clone()
        }
    }
}

/// An array of values as one document keyed by index.
fn indexed(values: &[Bson]) -> Document {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| (i.to_string(), v.clone()))
        .collect()
}

/// A non-empty container waiting to be opened: its document, its path and its value.
type Pending<'a> = (usize, Vec<String>, &'a Bson);

/// Every non-empty document or array that `flatten_doc` would show, breadth first across all
/// documents, until a cap is reached.
fn every_expandable(docs: &[Document]) -> TreeExpansion {
    let mut queue: VecDeque<Pending> = docs
        .iter()
        .enumerate()
        .flat_map(|(doc_index, doc)| children_of(doc_index, &[], shown_fields(doc)))
        .collect();
    let mut open = Expanded::new();
    let (mut paths, mut rows) = (0usize, 0usize);
    while let Some((doc_index, path, value)) = queue.pop_front() {
        let children = shown_children(value);
        if paths == MAX_EXPANDED_PATHS || rows + children.len() > MAX_EXPAND_ALL_ROWS {
            return TreeExpansion {
                open: Arc::new(open),
                capped: true,
            };
        }
        paths += 1;
        rows += children.len();
        queue.extend(children_of(doc_index, &path, children));
        open.entry(doc_index).or_default().insert(path);
    }
    TreeExpansion {
        open: Arc::new(open),
        capped: false,
    }
}

/// The entries that can be opened (non-empty documents and arrays), as queue items; scalars are
/// never queued.
fn children_of<'a>(
    doc_index: usize,
    parent: &[String],
    entries: Vec<(String, &'a Bson)>,
) -> Vec<Pending<'a>> {
    entries
        .into_iter()
        .filter(|(_, value)| is_openable(value))
        .map(|(key, value)| {
            let path = parent.iter().cloned().chain([key]).collect();
            (doc_index, path, value)
        })
        .collect()
}

/// A non-empty document or array (what `TreeRow::is_expandable` accepts).
fn is_openable(value: &Bson) -> bool {
    match value {
        Bson::Document(d) => !d.is_empty(),
        Bson::Array(a) => !a.is_empty(),
        _ => false,
    }
}

/// The fields of `doc` that `flatten_doc` lists (at most `MAX_ROWS_PER_LEVEL`).
fn shown_fields(doc: &Document) -> Vec<(String, &Bson)> {
    doc.iter()
        .take(MAX_ROWS_PER_LEVEL)
        .map(|(k, v)| (k.clone(), v))
        .collect()
}

/// The children of a document or array that `flatten_doc` lists when it is open (array keys
/// are indexes, as in the tree); empty for anything else.
fn shown_children(value: &Bson) -> Vec<(String, &Bson)> {
    match value {
        Bson::Document(d) => shown_fields(d),
        Bson::Array(a) => a
            .iter()
            .take(MAX_ROWS_PER_LEVEL)
            .enumerate()
            .map(|(i, v)| (i.to_string(), v))
            .collect(),
        _ => Vec::new(),
    }
}
