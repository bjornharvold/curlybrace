//! Reads a collection in ascending `_id` order, one batch at a time.
//!
//! The scan includes its start key, so a resumed read asks for one extra document and drops the
//! one equal to the resume key (the same keyset paging as `copy::engine`).
//!
//! What is bounded: a batch returned to the caller holds at most [`EXPORT_BATCH`] documents and,
//! unless it is a single oversized document, at most the byte budget (the documents that would
//! pass it are dropped and read again by the next batch). The size of a fetch adapts to the
//! largest document seen so far, so after the first fetch (a few documents) the documents held
//! while reading stay near the budget too. A document is never split: one bigger than the budget
//! is returned alone. Document values and `_id`s are never logged or put in an error.

use std::sync::Arc;

use bson::Document;

use crate::compare::key::{KeySpec, KeyTuple, compare_tuples, extract_key};
use crate::error::Result;
use crate::store::{DocumentStore, Namespace};

/// Documents per batch in production.
pub const EXPORT_BATCH: usize = 500;

/// Most BSON bytes a batch holds, unless it is one oversized document.
pub const EXPORT_BYTE_BUDGET: usize = 8 * 1024 * 1024;

/// Documents asked for before any document size is known.
const FIRST_FETCH: usize = 8;

/// Pages through the documents of one collection that match a filter.
pub struct DocReader {
    store: Arc<dyn DocumentStore>,
    ns: Namespace,
    filter: Document,
    batch: usize,
    byte_budget: usize,
    /// Documents asked for by the next fetch, adapted to the sizes seen.
    next_limit: usize,
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
            byte_budget: EXPORT_BYTE_BUDGET,
            next_limit: batch.clamp(1, FIRST_FETCH),
            key: KeySpec::id(),
            resume: None,
            finished: false,
        }
    }

    /// Replaces the byte budget of a batch (at least one byte).
    pub fn with_byte_budget(self, bytes: usize) -> Self {
        Self {
            byte_budget: bytes.max(1),
            ..self
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
        let wanted = self.next_limit + usize::from(self.resume.is_some());
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
        let sizes: Vec<usize> = docs.iter().map(encoded_size).collect();
        let keep = docs_within(&sizes, self.byte_budget);
        let more = may_have_more || keep < docs.len();
        let largest = sizes.iter().copied().max().unwrap_or(1).max(1);
        self.next_limit = (self.byte_budget / largest).clamp(1, self.batch);
        let docs: Vec<Document> = docs.into_iter().take(keep).collect();
        self.resume = match (more, docs.last()) {
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

/// The BSON size of a document; one that cannot be encoded counts as a whole budget.
fn encoded_size(doc: &Document) -> usize {
    bson::RawDocumentBuf::from_document(doc).map_or(usize::MAX, |raw| raw.as_bytes().len())
}

/// How many leading documents fit in `budget` together; at least one when there is any.
fn docs_within(sizes: &[usize], budget: usize) -> usize {
    let mut total = 0_usize;
    let mut keep = 0;
    for size in sizes {
        total = total.saturating_add(*size);
        if keep > 0 && total > budget {
            break;
        }
        keep += 1;
    }
    keep
}
