//! View state shared by the document views: expansion, delete confirmation, caches.
use super::doc_actions::{DeleteConfirm, DocAction, MenuItem, choose};
use super::json_cache::JsonCache;
use super::page_key::PageKey;
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
    /// The page is shown with a projection: Edit and Clone are not offered. Set by the app
    /// every frame from the query that produced the page.
    pub projected: bool,
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
            projected: self.projected,
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

    /// Context-menu choice on `doc` of the page keyed `key`; Delete only parks a confirmation.
    pub fn menu_chosen(
        self,
        item: MenuItem,
        doc: &Document,
        key: &PageKey,
    ) -> (Self, Option<DocAction>) {
        let (confirm, action) = choose(self.confirm.clone(), item, doc, *key);
        (Self { confirm, ..self }, action)
    }

    /// Emits the parked delete once, and only while `key` is the page it was requested on.
    pub fn delete_confirmed(self, key: &PageKey) -> (Self, Option<DocAction>) {
        let (confirm, action) = self.confirm.clone().confirmed(key);
        (Self { confirm, ..self }, action)
    }

    pub fn delete_cancelled(self) -> Self {
        Self {
            confirm: self.confirm.cancelled(),
            ..self
        }
    }

    /// The `_id` awaiting delete confirmation on the page keyed `key`, if any.
    pub fn pending_delete_id(&self, key: &PageKey) -> Option<&Bson> {
        self.confirm.pending_id(key)
    }

    /// Safety net independent of `results_replaced`: when the (content-sensitive) page key
    /// changed, drop expansion, cached rows and any pending delete of the old page.
    /// `show_documents` calls this once per frame, for every view mode.
    pub fn synced_to(&mut self, key: &PageKey) {
        let key = *key;
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
    pub fn page_rows(&mut self, page: &Page, page_id: &PageKey) -> Rc<Vec<PageRow>> {
        self.synced_to(page_id);
        let key = (self.generation, self.revision, *page_id);
        match &self.rows {
            Some((k, rows)) if *k == key => return Rc::clone(rows),
            _ => {}
        }
        let rows = Rc::new(flatten_page(&page.docs, &self.expanded));
        self.rows = Some((key, Rc::clone(&rows)));
        rows
    }

    /// JSON lines for the page, serialised once per generation and page identity.
    pub fn json_cache(&mut self, page: &Page, key: &PageKey) -> &JsonCache {
        self.synced_to(key);
        self.json = std::mem::take(&mut self.json).refreshed(self.generation, *key, page);
        &self.json
    }
}

#[cfg(test)]
mod tests {
    use super::super::json_cache::JsonLine;
    use super::super::page_key::page_key;
    use super::*;
    use bson::doc;
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

    fn rows_of(state: &mut TreeState, page: &Page) -> Rc<Vec<PageRow>> {
        state.page_rows(page, &page_key(page))
    }

    fn json_of(state: &mut TreeState, page: &Page) -> Rc<Vec<JsonLine>> {
        state.json_cache(page, &page_key(page)).shared_lines()
    }

    fn pending_of(state: &TreeState, page: &Page) -> Option<Bson> {
        state.pending_delete_id(&page_key(page)).cloned()
    }

    fn choose_delete(state: TreeState, page: &Page, index: usize) -> TreeState {
        state
            .menu_chosen(MenuItem::Delete, &page.docs[index], &page_key(page))
            .0
    }

    #[test]
    fn expansion_is_per_document() {
        let page = nested_page();
        let mut state = TreeState::default().expanded_toggled(1, &n_path());
        // 2 headers + 2*2 fields + 1 child, only in document 1.
        assert_eq!(rows_of(&mut state, &page).len(), 7);
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
        let a = rows_of(&mut state, &page);
        assert!(Rc::ptr_eq(&a, &rows_of(&mut state, &page)));
        let mut state = state.expanded_toggled(0, &n_path());
        let b = rows_of(&mut state, &page);
        assert!(!Rc::ptr_eq(&a, &b));
        let mut state = state.results_replaced();
        assert!(!Rc::ptr_eq(&b, &rows_of(&mut state, &page)));
    }

    fn id_page(ids: &[i32]) -> Page {
        page_of(ids.iter().map(|i| mock_doc_with_id(*i)).collect())
    }

    #[test]
    fn new_results_clear_pending_delete_and_expansion() {
        let page = id_page(&[5]);
        let state = choose_delete(
            TreeState::default().expanded_toggled(0, &n_path()),
            &page,
            0,
        );
        assert_eq!(pending_of(&state, &page), Some(Bson::Int32(5)));
        let state = state.results_replaced();
        assert_eq!(state.confirm, DeleteConfirm::None);
        assert!(!state.is_expanded(0, &n_path()));
        assert_eq!(state.generation(), 1);
    }

    #[test]
    fn confirm_flow_emits_once_and_cancel_clears() {
        let page = id_page(&[5]);
        let key = page_key(&page);
        let state = choose_delete(TreeState::default(), &page, 0);
        let (state, first) = state.delete_confirmed(&key);
        assert_eq!(first, Some(DocAction::Delete(Bson::Int32(5))));
        assert_eq!(state.delete_confirmed(&key).1, None);
        let state = choose_delete(TreeState::default(), &page, 0);
        assert_eq!(state.delete_cancelled().confirm, DeleteConfirm::None);
    }

    #[test]
    fn confirm_after_the_page_changed_without_results_replaced_emits_nothing() {
        let old = id_page(&[5]);
        let new = id_page(&[6]);
        let state = choose_delete(TreeState::default(), &old, 0);
        assert_eq!(pending_of(&state, &new), None);
        let (state, action) = state.delete_confirmed(&page_key(&new));
        assert_eq!(action, None);
        assert_eq!(state.confirm, DeleteConfirm::None);
    }

    #[test]
    fn different_page_same_generation_rebuilds_rows_and_drops_expansion() {
        let big = nested_page();
        let small = id_page(&[9]);
        let mut state = TreeState::default();
        let _ = rows_of(&mut state, &big);
        state = state.expanded_toggled(1, &n_path());
        assert_eq!(rows_of(&mut state, &big).len(), 7);
        let rows = rows_of(&mut state, &small);
        assert!(rows.iter().all(|r| r.doc_index() < small.docs.len()));
        assert_eq!(rows.len(), 1 + small.docs[0].len());
        assert!(!state.is_expanded(1, &n_path()));
    }

    #[test]
    fn json_cache_rebuilds_for_a_different_page_in_the_same_generation() {
        let mut state = TreeState::default();
        let first = page_of(vec![doc! { "_id": 1_i32, "a": 1_i32 }]);
        assert!(json_of(&mut state, &first).len() > 1);
        let second = page_of(vec![doc! { "_id": 2_i32, "z": 1_i32 }]);
        let fresh = json_of(&mut state, &second);
        assert!(fresh.iter().any(|l| l.text.contains("\"z\"")));
        assert!(fresh.iter().all(|l| !l.text.contains("\"a\"")));
    }

    #[test]
    fn identical_content_reuses_both_caches() {
        let page = nested_page();
        let mut state = TreeState::default();
        let rows = rows_of(&mut state, &page);
        let json = json_of(&mut state, &page);
        let copy = page.clone();
        assert!(Rc::ptr_eq(&rows, &rows_of(&mut state, &copy)));
        assert!(Rc::ptr_eq(&json, &json_of(&mut state, &copy)));
    }

    fn scalar_text(rows: &[PageRow], key: &str) -> String {
        rows.iter()
            .find_map(|r| match r {
                PageRow::Field { row, .. } if row.key == key => Some(row.value.clone()),
                _ => None,
            })
            .expect("row present")
    }

    #[test]
    fn value_only_refetch_rebuilds_rows_and_json_without_results_replaced() {
        let before = page_of(vec![doc! { "_id": 1_i32, "name": "alice" }]);
        let after = page_of(vec![doc! { "_id": 1_i32, "name": "alicia" }]);
        let mut state = TreeState::default();
        let rows = rows_of(&mut state, &before);
        assert_eq!(scalar_text(&rows, "name"), "alice");
        assert!(
            json_of(&mut state, &before)
                .iter()
                .any(|l| l.text.contains("alice"))
        );
        let rows = rows_of(&mut state, &after);
        assert_eq!(scalar_text(&rows, "name"), "alicia");
        let json = json_of(&mut state, &after);
        assert!(json.iter().any(|l| l.text.contains("alicia")));
        assert!(json.iter().all(|l| !l.text.contains("alice\"")));
    }

    #[test]
    fn value_change_in_a_doc_without_id_rebuilds_rows() {
        let before = page_of(vec![doc! { "n": 1_i32 }]);
        let after = page_of(vec![doc! { "n": 2_i32 }]);
        let mut state = TreeState::default();
        assert_eq!(scalar_text(&rows_of(&mut state, &before), "n"), "1");
        assert_eq!(scalar_text(&rows_of(&mut state, &after), "n"), "2");
    }

    #[test]
    fn int32_to_int64_with_same_number_rebuilds_rows() {
        let before = page_of(vec![doc! { "_id": 1_i32, "n": 1_i32 }]);
        let after = page_of(vec![doc! { "_id": 1_i32, "n": 1_i64 }]);
        let mut state = TreeState::default();
        let a = rows_of(&mut state, &before);
        assert!(!Rc::ptr_eq(&a, &rows_of(&mut state, &after)));
    }

    #[test]
    fn pending_delete_is_invalidated_by_a_value_only_change_with_the_same_id() {
        let before = page_of(vec![doc! { "_id": 1_i32, "name": "alice" }]);
        let after = page_of(vec![doc! { "_id": 1_i32, "name": "alicia" }]);
        let mut state = choose_delete(TreeState::default(), &before, 0);
        assert_eq!(pending_of(&state, &before), Some(Bson::Int32(1)));
        assert_eq!(pending_of(&state, &after), None);
        state.synced_to(&page_key(&after));
        assert_eq!(state.confirm, DeleteConfirm::None);
        let (_, action) =
            choose_delete(TreeState::default(), &before, 0).delete_confirmed(&page_key(&after));
        assert_eq!(action, None);
    }
}
