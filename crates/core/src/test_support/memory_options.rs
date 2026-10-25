//! Collection options of the in-memory store: what `create_collection` was given, per namespace.
//! The `DocumentStore` methods in `memory_store` delegate here.

use std::collections::BTreeMap;

use super::memory_indexes::NsKey;
use crate::error::{CoreError, Result};
use crate::store::CollectionOptions;

/// The options given to `create_collection`. A collection made by an insert has no entry and
/// reads back as the default.
pub(super) type OptionsMap = BTreeMap<NsKey, CollectionOptions>;

/// The options of a collection, `None` when it does not exist (`exists` is whether it holds a
/// data entry).
pub(super) fn get(map: &OptionsMap, k: &NsKey, exists: bool) -> Option<CollectionOptions> {
    exists.then(|| map.get(k).cloned().unwrap_or_default())
}

/// Records `options` for a new collection; fails when the collection exists.
pub(super) fn create(
    map: &mut OptionsMap,
    k: NsKey,
    exists: bool,
    options: &CollectionOptions,
) -> Result<()> {
    if exists {
        return Err(CoreError::Database("collection already exists".into()));
    }
    map.insert(k, options.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use bson::doc;

    use super::super::memory_store::InMemoryStore;
    use super::super::mock_objects::mock_namespace;
    use crate::error::CoreError;
    use crate::store::{CappedSpec, CollectionOptions, DocumentStore};

    fn rich_options() -> CollectionOptions {
        CollectionOptions {
            capped: Some(CappedSpec {
                size: 4096,
                max: Some(10),
            }),
            validator: Some(doc! { "$jsonSchema": { "required": ["a"] } }),
            validation_level: Some("moderate".into()),
            validation_action: Some("warn".into()),
            collation: Some(doc! { "locale": "fr" }),
        }
    }

    #[tokio::test]
    async fn create_collection_then_collection_options_returns_them() {
        let store = InMemoryStore::new();
        let ns = mock_namespace();
        store.create_collection(&ns, &rich_options()).await.unwrap();
        assert_eq!(
            store.collection_options(&ns).await.unwrap(),
            Some(rich_options())
        );
        assert_eq!(store.list_collections(&ns.db).await.unwrap(), ["people"]);
    }

    #[tokio::test]
    async fn collection_options_of_a_missing_collection_is_none() {
        let store = InMemoryStore::new();
        assert_eq!(
            store.collection_options(&mock_namespace()).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn an_implicitly_created_collection_has_default_options() {
        let ns = mock_namespace();
        let store = InMemoryStore::new();
        store.insert(&ns, doc! { "_id": 1 }).await.unwrap();
        assert_eq!(
            store.collection_options(&ns).await.unwrap(),
            Some(CollectionOptions::default())
        );
    }

    #[tokio::test]
    async fn create_collection_on_an_existing_collection_fails() {
        let ns = mock_namespace();
        let store = InMemoryStore::new();
        store
            .create_collection(&ns, &CollectionOptions::default())
            .await
            .unwrap();
        let again = store.create_collection(&ns, &rich_options()).await;
        assert!(matches!(again, Err(CoreError::Database(_))));
        assert_eq!(
            store.collection_options(&ns).await.unwrap(),
            Some(CollectionOptions::default()),
            "the first options stay"
        );

        let seeded = InMemoryStore::new().with_docs(&ns, vec![doc! { "_id": 1 }]);
        let over = seeded.create_collection(&ns, &rich_options()).await;
        assert!(matches!(over, Err(CoreError::Database(_))));
    }
}
