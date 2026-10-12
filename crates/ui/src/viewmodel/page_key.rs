//! Cheap identity of a result page, so caches and pending actions can never outlive it.
use mongostudio_core::store::Page;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageKey {
    pub len: usize,
    pub skip: u64,
    /// Hash of every document's `_id` (or of its field names when it has none).
    pub fingerprint: u64,
}

/// Identity of `page`: size, offset and a hash of all `_id`s (pages are small).
pub fn page_key(page: &Page) -> PageKey {
    let mut hasher = DefaultHasher::new();
    for doc in &page.docs {
        match doc.get("_id") {
            Some(id) => format!("{id:?}").hash(&mut hasher),
            None => doc.keys().collect::<Vec<_>>().hash(&mut hasher),
        }
        doc.len().hash(&mut hasher);
    }
    PageKey {
        len: page.docs.len(),
        skip: page.skip,
        fingerprint: hasher.finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    fn page(docs: Vec<bson::Document>, skip: u64) -> Page {
        Page {
            docs,
            skip,
            has_more: false,
        }
    }

    #[test]
    fn same_page_same_key() {
        let p = page(vec![doc! { "_id": 1_i32 }, doc! { "_id": "a" }], 0);
        assert_eq!(page_key(&p), page_key(&p.clone()));
    }

    #[test]
    fn differs_by_ids_len_skip_and_id_type() {
        let base = page(vec![doc! { "_id": 1_i32 }], 0);
        let other_id = page(vec![doc! { "_id": 2_i32 }], 0);
        let other_type = page(vec![doc! { "_id": 1_i64 }], 0);
        let longer = page(vec![doc! { "_id": 1_i32 }, doc! { "_id": 2_i32 }], 0);
        let skipped = page(vec![doc! { "_id": 1_i32 }], 5);
        for p in [other_id, other_type, longer, skipped] {
            assert_ne!(page_key(&base), page_key(&p));
        }
    }

    #[test]
    fn documents_without_id_are_fingerprinted_by_shape() {
        let a = page(vec![doc! { "x": 1_i32 }], 0);
        let b = page(vec![doc! { "y": 1_i32 }], 0);
        assert_ne!(page_key(&a), page_key(&b));
    }
}
