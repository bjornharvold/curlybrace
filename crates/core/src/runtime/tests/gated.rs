//! A store whose `find` signals when it started and waits for a release, shared by the
//! session and race tests.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use bson::{Bson, Document};
use tokio::sync::Notify;

use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace, Page, Query};

/// Find that signals when it started and waits for a release.
pub(crate) struct GatedFindStore {
    pub(crate) finished: Arc<AtomicUsize>,
    started: Arc<Notify>,
    release: Arc<Notify>,
}

pub(crate) fn down<T>() -> Result<T> {
    Err(CoreError::Database("down".into()))
}

#[async_trait]
impl DocumentStore for GatedFindStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, q: &Query) -> Result<Page> {
        self.started.notify_one();
        self.release.notified().await;
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(Page {
            docs: vec![],
            skip: q.skip,
            has_more: false,
        })
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        down()
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        down()
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        down()
    }
}

/// The store plus its `started` and `release` handles.
pub(crate) fn gated_find() -> (Arc<GatedFindStore>, Arc<Notify>, Arc<Notify>) {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let store = Arc::new(GatedFindStore {
        started: started.clone(),
        release: release.clone(),
        finished: Arc::new(AtomicUsize::new(0)),
    });
    (store, started, release)
}
