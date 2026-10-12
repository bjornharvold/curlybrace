//! Cached JSON view text so large pages are serialised once per result set.
use mongostudio_core::ejson::doc_to_view_json;
use mongostudio_core::store::Page;
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
    generation: Option<u64>,
    lines: Rc<Vec<JsonLine>>,
}

impl JsonCache {
    /// Rebuilt only when `generation` differs from the cached one.
    pub fn refreshed(self, generation: u64, page: &Page) -> Self {
        if self.generation == Some(generation) {
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
            generation: Some(generation),
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

    #[test]
    fn same_generation_keeps_cache_new_generation_rebuilds() {
        let first = page(vec![doc! { "a": 1_i32 }]);
        let second = page(vec![doc! { "z": 9_i32 }]);
        let cache = JsonCache::default().refreshed(1, &first);
        let kept = cache.clone().refreshed(1, &second);
        assert_eq!(kept, cache, "same generation must not re-serialise");
        let rebuilt = kept.refreshed(2, &second);
        assert!(rebuilt.lines().iter().any(|l| l.text.contains("\"z\"")));
        assert!(rebuilt.lines().iter().all(|l| !l.text.contains("\"a\"")));
    }

    #[test]
    fn lines_are_tagged_with_their_document() {
        let p = page(vec![doc! { "a": 1_i32 }, doc! { "b": 2_i32 }]);
        let cache = JsonCache::default().refreshed(0, &p);
        assert!(cache.lines().iter().any(|l| l.doc_index == 1));
        assert_eq!(cache.lines()[0].doc_index, 0);
    }
}
