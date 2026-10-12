//! Storage-agnostic document access: the seam between the UI and MongoDB.

use async_trait::async_trait;
use bson::{Bson, Document};

use crate::error::Result;

/// Documents fetched per page in the browser.
pub const PAGE_SIZE: u64 = 50;

/// A database + collection pair. Names may contain dots, spaces or unicode.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Namespace {
    pub db: String,
    pub collection: String,
}

impl Namespace {
    pub fn new(db: impl Into<String>, collection: impl Into<String>) -> Self {
        Self {
            db: db.into(),
            collection: collection.into(),
        }
    }
}

/// A find request: filter, optional sort/projection, and a skip/limit window.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub filter: Document,
    pub sort: Option<Document>,
    pub projection: Option<Document>,
    pub skip: u64,
    pub limit: u64,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            filter: Document::new(),
            sort: None,
            projection: None,
            skip: 0,
            limit: PAGE_SIZE,
        }
    }
}

/// One page of results. `has_more` is true when documents remain past `docs`.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub docs: Vec<Document>,
    pub skip: u64,
    pub has_more: bool,
}

#[async_trait]
pub trait DocumentStore: Send + Sync {
    async fn list_databases(&self) -> Result<Vec<String>>;
    async fn list_collections(&self, db: &str) -> Result<Vec<String>>;
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page>;
    /// Inserts `doc`, returning its `_id` (generated when absent).
    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson>;
    /// Replaces the document with `_id == id`; `NotFound` if none matches.
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()>;
    /// Deletes the document with `_id == id`; `NotFound` if none matches.
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_default_is_first_page_with_empty_filter() {
        let q = Query::default();
        assert!(q.filter.is_empty());
        assert!(q.sort.is_none() && q.projection.is_none());
        assert_eq!((q.skip, q.limit), (0, PAGE_SIZE));
    }
}
