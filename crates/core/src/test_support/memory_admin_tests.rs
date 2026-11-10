//! Administration in the in-memory store: it must behave like a real server where tests rely on it.

use bson::doc;

use super::memory_store::InMemoryStore;
use crate::error::CoreError;
use crate::indexes::IndexSpec;
use crate::store::{DocumentStore, Namespace};

fn spec(name: &str) -> IndexSpec {
    IndexSpec::from_document(&doc! { "name": name, "key": { "a": 1 } }).unwrap()
}

#[tokio::test]
async fn renaming_over_a_target_replaces_its_indexes_too() {
    let src = Namespace::new("shop", "src");
    let dst = Namespace::new("shop", "dst");
    let store = InMemoryStore::new()
        .with_docs(&src, vec![doc! { "_id": 1 }])
        .with_docs(&dst, vec![doc! { "_id": 2 }]);
    store.create_index(&dst, &spec("old_idx")).await.unwrap();
    store.rename_collection(&src, "dst", true).await.unwrap();
    let kept = store.list_index_specs(&dst).await.unwrap();
    assert!(
        kept.iter().all(|s| s.name != "old_idx"),
        "the replaced target's index must not survive: {kept:?}"
    );
}

#[tokio::test]
async fn collection_level_work_is_refused_in_a_server_owned_database() {
    let ns = Namespace::new("config", "chunks");
    let store = InMemoryStore::new().with_docs(&ns, vec![doc! { "_id": 1 }]);
    assert_eq!(
        store.drop_collection(&ns).await,
        Err(CoreError::ReservedDatabase)
    );
    assert_eq!(
        store.rename_collection(&ns, "x", false).await,
        Err(CoreError::ReservedDatabase)
    );
    assert_eq!(
        store.duplicate_collection(&ns, "x").await,
        Err(CoreError::ReservedDatabase)
    );
}

#[tokio::test]
async fn a_legacy_named_database_can_be_dropped() {
    let ns = Namespace::new("a:b", "c");
    let store = InMemoryStore::new().with_docs(&ns, vec![doc! { "_id": 1 }]);
    assert_eq!(store.drop_database("a:b").await, Ok(()));
}
