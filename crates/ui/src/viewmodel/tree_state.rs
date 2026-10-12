//! View state shared by the document views: expansion, delete confirmation, caches.
use super::doc_actions::{DeleteConfirm, DocAction, MenuItem, choose};
use super::json_cache::JsonCache;
use super::page_key::{PageKey, page_key};
use super::tree::{TreeRow, flatten_doc};
use bson::{Bson, Document};
use mongostudio_core::store::Page;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// One line of the flattened page tree.
#[derive(Debug, Clone, PartialEq)]
pub enum PageRow {
    /// Heading of document `doc_index`.
    Header {
        doc_index: usize,
    },
    Field {
        doc_index: usize,
        row: TreeRow,
    },
}

impl PageRow {
    pub fn doc_index(&self) -> usize {
        match self {
            PageRow::Header { doc_index } | PageRow::Field { doc_index, .. } => *doc_index,
        }
    }
}

/// Flattens every document of `docs`, each preceded by its header.
pub fn flatten_page(
    docs: &[Document],
    expanded: &HashMap<usize, HashSet<Vec<String>>>,
) -> Vec<PageRow> {
    let none = HashSet::new();
    let mut out = Vec::new();
    for (doc_index, doc) in docs.iter().enumerate() {
        out.push(PageRow::Header { doc_index });
        let open = expanded.get(&doc_index).unwrap_or(&none);
        out.extend(
            flatten_doc(doc, open)
                .into_iter()
                .map(|row| PageRow::Field { doc_index, row }),
        );
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct TreeState {
    generation: u64,
    /// Bumped on every expansion change; part of the row-cache key.
    revision: u64,
    expanded: HashMap<usize, HashSet<Vec<String>>>,
    pub confirm: DeleteConfirm,
    json: JsonCache,
    rows: Option<(CacheKey, Rc<Vec<PageRow>>)>,
    /// Identity of the page the state last rendered.
    seen_page: Option<PageKey>,
}

/// `(generation, revision, page)` the cached rows were built for.
type CacheKey = (u64, u64, PageKey);

impl TreeState {
    /// Call when a new result page arrives: resets expansion, caches and any pending delete.
    pub fn results_replaced(self) -> Self {
        Self {
            generation: self.generation + 1,
            confirm: self.confirm.results_replaced(),
            ..Self::default()
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_expanded(&self, doc_index: usize, path: &[String]) -> bool {
        self.expanded
            .get(&doc_index)
            .is_some_and(|set| set.contains(path))
    }

    /// Flips the expansion of `path` inside document `doc_index`.
    pub fn expanded_toggled(self, doc_index: usize, path: &[String]) -> Self {
        let mut expanded = self.expanded;
        let set = expanded.entry(doc_index).or_default();
        if !set.remove(path) {
            set.insert(path.to_vec());
        }
        Self {
            expanded,
            revision: self.revision + 1,
            rows: None,
            ..self
        }
    }

    /// A click on a row toggles it only when it has children to show.
    pub fn row_clicked(self, doc_index: usize, row: &TreeRow) -> Self {
        if row.is_expandable() {
            self.expanded_toggled(doc_index, &row.path)
        } else {
            self
        }
    }

    /// Context-menu choice on `doc` of `page`; Delete only parks a confirmation.
    pub fn menu_chosen(
        self,
        item: MenuItem,
        doc: &Document,
        page: &Page,
    ) -> (Self, Option<DocAction>) {
        let (confirm, action) = choose(self.confirm.clone(), item, doc, page_key(page));
        (Self { confirm, ..self }, action)
    }

    /// Emits the parked delete once, and only while `page` is the page it was requested on.
    pub fn delete_confirmed(self, page: &Page) -> (Self, Option<DocAction>) {
        let (confirm, action) = self.confirm.clone().confirmed(&page_key(page));
        (Self { confirm, ..self }, action)
    }

    pub fn delete_cancelled(self) -> Self {
        Self {
            confirm: self.confirm.cancelled(),
            ..self
        }
    }

    /// The `_id` awaiting delete confirmation on `page`, if any.
    pub fn pending_delete_id(&self, page: &Page) -> Option<&Bson> {
        self.confirm.pending_id(&page_key(page))
    }

    /// Safety net independent of `results_replaced`: when the page identity changed,
    /// drop expansion, cached rows and any pending delete that belonged to the old page.
    fn synced_to(&mut self, key: PageKey) {
        match self.seen_page {
            Some(seen) if seen == key => return,
            Some(_) => {
                self.expanded.clear();
                self.revision += 1;
                self.rows = None;
            }
            None => {}
        }
        self.confirm = std::mem::take(&mut self.confirm).validated(&key);
        self.seen_page = Some(key);
    }

    /// The flattened page, rebuilt after a new result set, a different page or an expansion change.
    pub fn page_rows(&mut self, page: &Page) -> Rc<Vec<PageRow>> {
        let page_id = page_key(page);
        self.synced_to(page_id);
        let key = (self.generation, self.revision, page_id);
        match &self.rows {
            Some((k, rows)) if *k == key => return Rc::clone(rows),
            _ => {}
        }
        let rows = Rc::new(flatten_page(&page.docs, &self.expanded));
        self.rows = Some((key, Rc::clone(&rows)));
        rows
    }

    /// JSON lines for the page, serialised once per generation and page identity.
    pub fn json_cache(&mut self, page: &Page) -> &JsonCache {
        self.synced_to(page_key(page));
        self.json = std::mem::take(&mut self.json).refreshed(self.generation, page);
        &self.json
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Bson, doc};
    use mongostudio_core::test_support::mock_objects::mock_doc_with_id;

    fn page_of(docs: Vec<Document>) -> Page {
        Page {
            docs,
            skip: 0,
            has_more: false,
        }
    }

    fn nested_page() -> Page {
        page_of(vec![
            doc! { "_id": 1_i32, "n": { "a": 1_i32 } },
            doc! { "_id": 2_i32, "n": { "b": 1_i32 } },
        ])
    }

    fn n_path() -> Vec<String> {
        vec!["n".to_string()]
    }

    #[test]
    fn expansion_is_per_document() {
        let page = nested_page();
        let mut state = TreeState::default().expanded_toggled(1, &n_path());
        // 2 headers + 2*2 fields + 1 child, only in document 1.
        assert_eq!(state.page_rows(&page).len(), 7);
        assert!(state.is_expanded(1, &n_path()));
        assert!(!state.is_expanded(0, &n_path()));
    }

    #[test]
    fn toggling_twice_collapses_again() {
        let state = TreeState::default()
            .expanded_toggled(0, &n_path())
            .expanded_toggled(0, &n_path());
        assert!(!state.is_expanded(0, &n_path()));
    }

    #[test]
    fn click_toggles_only_expandable_rows() {
        let d = doc! { "n": { "a": 1_i32 }, "x": 1_i32, "e": {} };
        let rows = flatten_doc(&d, &HashSet::new());
        let state = TreeState::default().row_clicked(0, &rows[1]);
        assert_eq!(state.revision, 0, "scalar click is a no-op");
        let state = state.row_clicked(0, &rows[2]);
        assert_eq!(state.revision, 0, "empty container click is a no-op");
        let state = state.row_clicked(0, &rows[0]);
        assert!(state.is_expanded(0, &rows[0].path));
    }

    #[test]
    fn page_rows_are_cached_until_expansion_or_results_change() {
        let page = nested_page();
        let mut state = TreeState::default();
        let a = state.page_rows(&page);
        assert!(Rc::ptr_eq(&a, &state.page_rows(&page)));
        let mut state = state.expanded_toggled(0, &n_path());
        let b = state.page_rows(&page);
        assert!(!Rc::ptr_eq(&a, &b));
        let mut state = state.results_replaced();
        assert!(!Rc::ptr_eq(&b, &state.page_rows(&page)));
    }

    fn id_page(ids: &[i32]) -> Page {
        page_of(ids.iter().map(|i| mock_doc_with_id(*i)).collect())
    }

    #[test]
    fn new_results_clear_pending_delete_and_expansion() {
        let page = id_page(&[5]);
        let (state, action) = TreeState::default()
            .expanded_toggled(0, &n_path())
            .menu_chosen(MenuItem::Delete, &page.docs[0], &page);
        assert_eq!(action, None);
        assert_eq!(state.pending_delete_id(&page), Some(&Bson::Int32(5)));
        let state = state.results_replaced();
        assert_eq!(state.confirm, DeleteConfirm::None);
        assert!(!state.is_expanded(0, &n_path()));
        assert_eq!(state.generation(), 1);
    }

    #[test]
    fn confirm_flow_emits_once_and_cancel_clears() {
        let page = id_page(&[5]);
        let (state, _) = TreeState::default().menu_chosen(MenuItem::Delete, &page.docs[0], &page);
        let (state, first) = state.delete_confirmed(&page);
        assert_eq!(first, Some(DocAction::Delete(Bson::Int32(5))));
        assert_eq!(state.delete_confirmed(&page).1, None);
        let (state, _) = TreeState::default().menu_chosen(MenuItem::Delete, &page.docs[0], &page);
        assert_eq!(state.delete_cancelled().confirm, DeleteConfirm::None);
    }

    #[test]
    fn confirm_after_the_page_changed_without_results_replaced_emits_nothing() {
        let old = id_page(&[5]);
        let new = id_page(&[6]);
        let (state, _) = TreeState::default().menu_chosen(MenuItem::Delete, &old.docs[0], &old);
        assert_eq!(state.pending_delete_id(&new), None);
        let (state, action) = state.delete_confirmed(&new);
        assert_eq!(action, None);
        assert_eq!(state.confirm, DeleteConfirm::None);
    }

    #[test]
    fn different_page_same_generation_rebuilds_rows_and_drops_expansion() {
        let big = nested_page();
        let small = id_page(&[9]);
        let mut state = TreeState::default();
        let _ = state.page_rows(&big);
        state = state.expanded_toggled(1, &n_path());
        assert_eq!(state.page_rows(&big).len(), 7);
        let rows = state.page_rows(&small);
        assert!(rows.iter().all(|r| r.doc_index() < small.docs.len()));
        assert_eq!(rows.len(), 1 + small.docs[0].len());
        assert!(!state.is_expanded(1, &n_path()));
    }

    #[test]
    fn json_cache_rebuilds_for_a_different_page_in_the_same_generation() {
        let mut state = TreeState::default();
        let first = page_of(vec![doc! { "_id": 1_i32, "a": 1_i32 }]);
        assert!(state.json_cache(&first).lines().len() > 1);
        let second = page_of(vec![doc! { "_id": 2_i32, "z": 1_i32 }]);
        let fresh = state.json_cache(&second);
        assert!(fresh.lines().iter().any(|l| l.text.contains("\"z\"")));
        assert!(fresh.lines().iter().all(|l| !l.text.contains("\"a\"")));
    }

    #[test]
    fn same_page_and_generation_reuses_both_caches() {
        let page = nested_page();
        let mut state = TreeState::default();
        let rows = state.page_rows(&page);
        let json = state.json_cache(&page).shared_lines();
        assert!(Rc::ptr_eq(&rows, &state.page_rows(&page)));
        assert!(Rc::ptr_eq(&json, &state.json_cache(&page).shared_lines()));
    }
}
