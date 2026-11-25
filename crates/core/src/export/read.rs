//! Reads a collection in ascending `_id` order, one batch at a time.
//!
//! The scan includes its start key, so a resumed read asks for one extra document and drops the
//! one equal to the resume key (the same keyset paging as `copy::engine`). Memory is bounded by
//! the batch size. Document values and `_id`s are never logged or put in an error.

use std::sync::Arc;

use bson::Document;

use crate::compare::key::{KeySpec, KeyTuple, compare_tuples, extract_key};
use crate::error::Result;
use crate::store::{DocumentStore, Namespace};

/// Documents per batch in production.
pub const EXPORT_BATCH: usize = 500;

/// Pages through the documents of one collection that match a filter.
pub struct DocReader {
    store: Arc<dyn DocumentStore>,
    ns: Namespace,
    filter: Document,
    batch: usize,
    key: KeySpec,
    resume: Option<KeyTuple>,
    finished: bool,
}

impl DocReader {
    pub fn new(
        store: Arc<dyn DocumentStore>,
        ns: Namespace,
        filter: Document,
        batch: usize,
    ) -> Self {
        Self {
            store,
            ns,
            filter,
            batch: batch.max(1),
            key: KeySpec::id(),
            resume: None,
            finished: false,
        }
    }

    /// The next batch, or `None` once every document has been returned. After an error the read
    /// is over and later calls return `None`.
    pub async fn next_batch(&mut self) -> Result<Option<Vec<Document>>> {
        if self.finished {
            return Ok(None);
        }
        let result = self.fetch().await;
        if result.is_err() {
            self.finished = true;
        }
        result
    }

    async fn fetch(&mut self) -> Result<Option<Vec<Document>>> {
        let wanted = self.batch + usize::from(self.resume.is_some());
        let limit = u32::try_from(wanted).unwrap_or(u32::MAX);
        let fetched = self
            .store
            .scan_sorted(
                &self.ns,
                &self.filter,
                &self.key,
                self.resume.as_ref(),
                limit,
            )
            .await?;
        let may_have_more = fetched.len() >= wanted;
        let docs = drop_resumed(fetched, &self.key, self.resume.as_ref())?;
        self.resume = match (may_have_more, docs.last()) {
            (true, Some(last)) => Some(extract_key(last, &self.key)?),
            _ => None,
        };
        self.finished = self.resume.is_none();
        Ok(if docs.is_empty() { None } else { Some(docs) })
    }
}

/// Drops the document equal to `from`, which the previous batch already returned.
fn drop_resumed(
    mut docs: Vec<Document>,
    key: &KeySpec,
    from: Option<&KeyTuple>,
) -> Result<Vec<Document>> {
    if let (Some(from), Some(first)) = (from, docs.first()) {
        if compare_tuples(&extract_key(first, key)?, from).is_eq() {
            docs.remove(0);
        }
    }
    Ok(docs)
}
