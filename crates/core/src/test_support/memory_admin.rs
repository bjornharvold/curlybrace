//! Database and collection administration of the in-memory store. Views are not supported:
//! the store cannot run a pipeline. The `DocumentStore` methods in `memory_store` delegate here.

use super::memory_store::{InMemoryStore, poisoned};
use crate::admin::{
    CollectionStats, DatabaseStats, ensure_user_database, validate_collection_name,
    validate_existing_collection_name, validate_existing_database_name,
};
use crate::error::{CoreError, Result};
use crate::store::Namespace;

impl InMemoryStore {
    pub(super) fn admin_drop_collection(&self, ns: &Namespace) -> Result<()> {
        ensure_user_database(&ns.db)?;
        validate_existing_collection_name(&ns.collection)?;
        let key = (ns.db.clone(), ns.collection.clone());
        self.data.lock().map_err(|_| poisoned())?.remove(&key);
        self.options.lock().map_err(|_| poisoned())?.remove(&key);
        self.indexes.lock().map_err(|_| poisoned())?.remove(&key);
        Ok(())
    }

    pub(super) fn admin_drop_database(&self, db: &str) -> Result<()> {
        validate_existing_database_name(db)?;
        self.data
            .lock()
            .map_err(|_| poisoned())?
            .retain(|(d, _), _| d != db);
        self.options
            .lock()
            .map_err(|_| poisoned())?
            .retain(|(d, _), _| d != db);
        self.indexes
            .lock()
            .map_err(|_| poisoned())?
            .retain(|(d, _), _| d != db);
        Ok(())
    }

    pub(super) fn admin_rename(&self, ns: &Namespace, to: &str, drop_target: bool) -> Result<()> {
        ensure_user_database(&ns.db)?;
        validate_existing_collection_name(&ns.collection)?;
        validate_collection_name(to)?;
        let (from, target) = (
            (ns.db.clone(), ns.collection.clone()),
            (ns.db.clone(), to.to_owned()),
        );
        let mut data = self.data.lock().map_err(|_| poisoned())?;
        if !data.contains_key(&from) {
            return Err(CoreError::NotFound);
        }
        if from != target && data.contains_key(&target) && !drop_target {
            return Err(CoreError::NamespaceExists);
        }
        let docs = data.remove(&from).unwrap_or_default();
        data.insert(target.clone(), docs);
        // The target now is the source: whatever the replaced target had must not survive.
        let mut options = self.options.lock().map_err(|_| poisoned())?;
        options.remove(&target);
        if let Some(o) = options.remove(&from) {
            options.insert(target.clone(), o);
        }
        let mut indexes = self.indexes.lock().map_err(|_| poisoned())?;
        indexes.remove(&target);
        if let Some(i) = indexes.remove(&from) {
            indexes.insert(target, i);
        }
        Ok(())
    }

    pub(super) fn admin_duplicate(&self, ns: &Namespace, target: &str) -> Result<()> {
        ensure_user_database(&ns.db)?;
        validate_collection_name(target)?;
        let (from, to) = (
            (ns.db.clone(), ns.collection.clone()),
            (ns.db.clone(), target.to_owned()),
        );
        let mut data = self.data.lock().map_err(|_| poisoned())?;
        let docs = data.get(&from).cloned().ok_or(CoreError::NotFound)?;
        if data.contains_key(&to) {
            return Err(CoreError::NamespaceExists);
        }
        data.insert(to.clone(), docs);
        let mut options = self.options.lock().map_err(|_| poisoned())?;
        if let Some(o) = options.get(&from).cloned() {
            options.insert(to.clone(), o);
        }
        let mut indexes = self.indexes.lock().map_err(|_| poisoned())?;
        if let Some(i) = indexes.get(&from).cloned() {
            indexes.insert(to, i);
        }
        Ok(())
    }

    pub(super) fn admin_collection_stats(&self, ns: &Namespace) -> Result<CollectionStats> {
        let data = self.data.lock().map_err(|_| poisoned())?;
        let docs = data
            .get(&(ns.db.clone(), ns.collection.clone()))
            .ok_or(CoreError::NotFound)?;
        Ok(CollectionStats {
            document_count: docs.len() as u64,
            data_size: docs.iter().map(approx_size).sum(),
            ..CollectionStats::default()
        })
    }

    pub(super) fn admin_database_stats(&self, db: &str) -> Result<DatabaseStats> {
        let data = self.data.lock().map_err(|_| poisoned())?;
        let mut stats = DatabaseStats::default();
        for ((d, _), docs) in data.iter() {
            if d == db {
                stats.collections += 1;
                stats.document_count += docs.len() as u64;
                stats.data_size += docs.iter().map(approx_size).sum::<u64>();
            }
        }
        Ok(stats)
    }
}

/// The encoded size of a document, 0 when it cannot be encoded.
fn approx_size(doc: &bson::Document) -> u64 {
    bson::to_vec(doc).map_or(0, |bytes| bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use bson::doc;

    use super::*;
    use crate::store::DocumentStore;
    use crate::test_support::mock_objects::mock_namespace;

    fn seeded() -> InMemoryStore {
        InMemoryStore::new().with_docs(
            &mock_namespace(),
            vec![doc! { "_id": 1 }, doc! { "_id": 2 }],
        )
    }

    #[tokio::test]
    async fn dropping_a_collection_removes_it_and_dropping_it_again_is_done() {
        let store = seeded();
        let ns = mock_namespace();
        store.drop_collection(&ns).await.unwrap();
        assert!(store.list_collections(&ns.db).await.unwrap().is_empty());
        assert_eq!(store.drop_collection(&ns).await, Ok(()));
    }

    #[tokio::test]
    async fn dropping_a_database_removes_its_collections_and_refuses_server_databases() {
        let store = seeded();
        let ns = mock_namespace();
        store.drop_database(&ns.db).await.unwrap();
        assert!(store.list_collections(&ns.db).await.unwrap().is_empty());
        assert_eq!(
            store.drop_database("admin").await,
            Err(CoreError::ReservedDatabase)
        );
    }

    #[tokio::test]
    async fn renaming_moves_the_documents_and_respects_the_drop_target_choice() {
        let ns = mock_namespace();
        let store = seeded().with_docs(&Namespace::new(&ns.db, "taken"), vec![doc! { "_id": 9 }]);
        assert_eq!(
            store.rename_collection(&ns, "taken", false).await,
            Err(CoreError::NamespaceExists)
        );
        store.rename_collection(&ns, "taken", true).await.unwrap();
        assert_eq!(
            store
                .estimated_count(&Namespace::new(&ns.db, "taken"))
                .await
                .unwrap(),
            2
        );
        assert_eq!(store.list_collections(&ns.db).await.unwrap(), ["taken"]);
        assert_eq!(
            store.rename_collection(&ns, "x", false).await,
            Err(CoreError::NotFound)
        );
    }

    #[tokio::test]
    async fn duplicating_copies_the_documents_but_not_onto_an_existing_name() {
        let ns = mock_namespace();
        let store = seeded();
        store.duplicate_collection(&ns, "copy").await.unwrap();
        assert_eq!(
            store
                .estimated_count(&Namespace::new(&ns.db, "copy"))
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            store.duplicate_collection(&ns, "copy").await,
            Err(CoreError::NamespaceExists)
        );
    }

    #[tokio::test]
    async fn statistics_count_documents_and_collections() {
        let ns = mock_namespace();
        let store = seeded();
        let stats = store.collection_stats(&ns).await.unwrap();
        assert_eq!(stats.document_count, 2);
        assert!(stats.data_size > 0);
        assert_eq!(store.database_stats(&ns.db).await.unwrap().collections, 1);
        assert_eq!(
            store
                .collection_stats(&Namespace::new(&ns.db, "nope"))
                .await,
            Err(CoreError::NotFound)
        );
    }
}
