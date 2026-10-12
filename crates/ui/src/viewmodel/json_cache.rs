//! Cached JSON view text so large pages are serialised once per result set.
use super::page_key::PageKey;
use curlybrace_core::ejson::doc_to_view_json;
use curlybrace_core::store::Page;
use std::rc::Rc;

/// One pretty-printed relaxed extended JSON string per document.
pub fn view_json_cache(page: &Page) -> Vec<String> {
    page.docs.iter().map(doc_to_view_json).collect()
}

/// One displayed line, tagged with the document it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonLine {
    pub doc_index: usize,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonCache {
    key: Option<(u64, PageKey)>,
    lines: Rc<Vec<JsonLine>>,
}

impl JsonCache {
    /// Rebuilt when the caller's `generation` OR the page identity differs from the cached one.
    pub fn refreshed(self, generation: u64, page_id: PageKey, page: &Page) -> Self {
        let key = (generation, page_id);
        if self.key == Some(key) {
            return self;
        }
        let lines = view_json_cache(page)
            .iter()
            .enumerate()
            .flat_map(|(doc_index, text)| {
                text.lines().map(move |l| JsonLine {
                    doc_index,
                    text: l.to_string(),
                })
            })
            .collect();
        Self {
            key: Some(key),
            lines: Rc::new(lines),
        }
    }

    pub fn lines(&self) -> &[JsonLine] {
        &self.lines
    }

    /// Cheap shared handle, so a view can hold the lines while mutating its state.
    pub fn shared_lines(&self) -> Rc<Vec<JsonLine>> {
        Rc::clone(&self.lines)
    }
}

#[cfg(test)]
mod tests {
    use super::super::page_key::page_key;
    use super::*;
    use bson::doc;

    fn page(docs: Vec<bson::Document>) -> Page {
        Page {
            docs,
            skip: 0,
            has_more: false,
        }
    }

    #[test]
    fn one_json_string_per_doc() {
        let p = page(vec![doc! { "a": 1_i32 }, doc! { "b": 2_i32 }]);
        let json = view_json_cache(&p);
        assert_eq!(json.len(), 2);
        assert!(json[1].contains("\"b\""));
    }

    fn refresh(cache: JsonCache, generation: u64, p: &Page) -> JsonCache {
        cache.refreshed(generation, page_key(p), p)
    }

    #[test]
    fn same_generation_and_page_reuses_the_cache() {
        let p = page(vec![doc! { "_id": 1_i32, "a": 1_i32 }]);
        let cache = refresh(JsonCache::default(), 1, &p);
        let before = cache.shared_lines();
        let again = refresh(cache, 1, &p.clone());
        assert!(Rc::ptr_eq(&before, &again.shared_lines()));
    }

    #[test]
    fn new_generation_rebuilds() {
        let p = page(vec![doc! { "_id": 1_i32 }]);
        let cache = refresh(JsonCache::default(), 1, &p);
        let before = cache.shared_lines();
        let rebuilt = refresh(cache, 2, &p);
        assert!(!Rc::ptr_eq(&before, &rebuilt.shared_lines()));
    }

    #[test]
    fn same_generation_but_different_page_rebuilds() {
        let first = page(vec![doc! { "_id": 1_i32, "a": 1_i32 }]);
        let second = page(vec![doc! { "_id": 2_i32, "z": 9_i32 }]);
        let cache = refresh(JsonCache::default(), 1, &first);
        let rebuilt = refresh(cache, 1, &second);
        assert!(rebuilt.lines().iter().any(|l| l.text.contains("\"z\"")));
        assert!(rebuilt.lines().iter().all(|l| !l.text.contains("\"a\"")));
    }

    #[test]
    fn lines_are_tagged_with_their_document() {
        let p = page(vec![doc! { "a": 1_i32 }, doc! { "b": 2_i32 }]);
        let cache = refresh(JsonCache::default(), 0, &p);
        assert!(cache.lines().iter().any(|l| l.doc_index == 1));
        assert_eq!(cache.lines()[0].doc_index, 0);
    }
}
