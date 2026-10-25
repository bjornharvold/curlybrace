//! A `DocumentStore` wrapper that refuses writes while a shared flag is set.
//!
//! Every trait method is forwarded explicitly, defaulted ones included: leaving one out would
//! let the trait default silently replace the inner store's real behaviour.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use bson::{Bson, Document};

use crate::compare::key::{KeySpec, KeyTuple};
use crate::error::{CoreError, Result};
use crate::indexes::IndexSpec;
use crate::store::{
    CollectionIndex, CollectionKind, CollectionOptions, DocumentStore, DuplicateGroup, IndexStatus,
    IndexUsage, Namespace, Page, Query, Topology, WriteOp, WriteOutcome,
};

pub struct ReadOnlyStore {
    inner: Arc<dyn DocumentStore>,
    locked: Arc<AtomicBool>,
}

impl ReadOnlyStore {
    /// Wraps `inner`; writes fail with `CoreError::ReadOnly` whenever `locked` is true.
    pub fn wrap(inner: Arc<dyn DocumentStore>, locked: Arc<AtomicBool>) -> Arc<dyn DocumentStore> {
        Arc::new(Self { inner, locked })
    }

    fn guard(&self) -> Result<()> {
        if self.locked.load(Ordering::SeqCst) {
            Err(CoreError::ReadOnly)
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl DocumentStore for ReadOnlyStore {
    fn is_read_only(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases().await
    }

    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }

    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.inner.find(ns, q).await
    }

    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        self.guard()?;
        self.inner.insert(ns, doc).await
    }

    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        self.guard()?;
        self.inner.replace(ns, id, doc).await
    }

    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.guard()?;
        self.inner.delete(ns, id).await
    }

    async fn ping(&self) -> Result<()> {
        self.inner.ping().await
    }

    async fn topology(&self) -> Result<Option<Topology>> {
        self.inner.topology().await
    }

    async fn server_version(&self) -> Result<Option<String>> {
        self.inner.server_version().await
    }

    async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        self.guard()?;
        self.inner.bulk_write(ns, ops).await
    }

    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        self.inner.estimated_count(ns).await
    }

    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        self.inner.scan_sorted(ns, filter, key, from, limit).await
    }

    async fn scan_sorted_primary(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        self.inner
            .scan_sorted_primary(ns, filter, key, from, limit)
            .await
    }

    async fn key_index_status(&self, ns: &Namespace, key: &KeySpec) -> Result<IndexStatus> {
        self.inner.key_index_status(ns, key).await
    }

    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        self.inner.list_collection_kinds(db).await
    }

    async fn duplicate_keys(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        limit: u32,
    ) -> Result<Vec<DuplicateGroup>> {
        self.inner.duplicate_keys(ns, filter, key, limit).await
    }

    async fn list_indexes(&self, ns: &Namespace) -> Result<Vec<CollectionIndex>> {
        self.inner.list_indexes(ns).await
    }

    async fn list_index_specs(&self, ns: &Namespace) -> Result<Vec<IndexSpec>> {
        self.inner.list_index_specs(ns).await
    }

    async fn index_stats(&self, ns: &Namespace) -> Result<Vec<IndexUsage>> {
        self.inner.index_stats(ns).await
    }

    async fn create_index(&self, ns: &Namespace, spec: &IndexSpec) -> Result<()> {
        self.guard()?;
        self.inner.create_index(ns, spec).await
    }

    async fn drop_index(&self, ns: &Namespace, name: &str) -> Result<()> {
        self.guard()?;
        self.inner.drop_index(ns, name).await
    }

    async fn set_index_hidden(&self, ns: &Namespace, name: &str, hidden: bool) -> Result<()> {
        self.guard()?;
        self.inner.set_index_hidden(ns, name, hidden).await
    }

    async fn set_index_ttl(&self, ns: &Namespace, name: &str, seconds: i64) -> Result<()> {
        self.guard()?;
        self.inner.set_index_ttl(ns, name, seconds).await
    }

    async fn create_collection(&self, ns: &Namespace, options: &CollectionOptions) -> Result<()> {
        self.guard()?;
        self.inner.create_collection(ns, options).await
    }

    async fn collection_options(&self, ns: &Namespace) -> Result<Option<CollectionOptions>> {
        self.inner.collection_options(ns).await
    }
}
