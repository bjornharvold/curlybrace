//! Cheap identity of a result page, so caches and pending actions can never outlive it.
use mongostudio_core::store::Page;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PageKey {
    pub len: usize,
    pub skip: u64,
    /// Hash of the full BSON content of every document (types and values included).
    pub fingerprint: u64,
}

/// Marker folded into the hash when a document cannot be serialised.
const UNSERIALISABLE_MARKER: u8 = 0xFF;

/// Content-sensitive identity of `page`: size, offset and a hash of every document's BSON
/// bytes (pages are small). Compute it once per frame and pass it down.
pub fn page_key(page: &Page) -> PageKey {
    let mut hasher = DefaultHasher::new();
    for doc in &page.docs {
        match bson::to_vec(doc) {
            Ok(bytes) => bytes.hash(&mut hasher),
            Err(_) => {
                UNSERIALISABLE_MARKER.hash(&mut hasher);
                doc.keys().collect::<Vec<_>>().hash(&mut hasher);
            }
        }
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

    #[test]
    fn value_only_change_with_same_ids_changes_the_key() {
        let before = page(vec![doc! { "_id": 1_i32, "name": "alice" }], 0);
        let after = page(vec![doc! { "_id": 1_i32, "name": "alicia" }], 0);
        assert_ne!(page_key(&before), page_key(&after));
    }

    #[test]
    fn value_change_in_a_doc_without_id_changes_the_key() {
        let before = page(vec![doc! { "n": 1_i32 }], 0);
        let after = page(vec![doc! { "n": 2_i32 }], 0);
        assert_ne!(page_key(&before), page_key(&after));
    }

    #[test]
    fn int32_vs_int64_of_the_same_number_differ() {
        let a = page(vec![doc! { "_id": 1_i32, "n": 1_i32 }], 0);
        let b = page(vec![doc! { "_id": 1_i32, "n": 1_i64 }], 0);
        assert_ne!(page_key(&a), page_key(&b));
    }

    #[test]
    fn identical_content_has_an_equal_key() {
        let a = page(vec![doc! { "_id": 1_i32, "n": [1_i32, { "k": "v" }] }], 3);
        assert_eq!(page_key(&a), page_key(&a.clone()));
    }
}
