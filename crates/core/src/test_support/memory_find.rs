//! The find side of the in-memory store: query evaluation, a record of every query that reached
//! `find`, `explain_find` and `explain_plan` (with the operation), the same refusal of a hint
//! naming no index that MongoDB gives, and one scripted find failure.

use std::sync::MutexGuard;

use bson::Document;

use super::memory_indexes as idx;
use super::memory_store::{InMemoryStore, key, poisoned};
use super::query_eval::{check_filter, compare, matches, parse_sort, project};
use crate::error::{CoreError, Result};
use crate::pipeline_options::HintOption;
use crate::query_options::FindOperation;
use crate::store::{Namespace, Page, Query};

/// What reached the find operations, and the failure they answer with when one is scripted.
#[derive(Default)]
pub(super) struct FindScript {
    received: Vec<(FindOperation, Query)>,
    error: Option<CoreError>,
}

impl InMemoryStore {
    /// The queries that reached `find`, `explain_find` and `explain_plan`, in call order, each
    /// with the operation. A query refused for its hint is recorded too.
    pub fn received_queries(&self) -> Vec<(FindOperation, Query)> {
        self.find_script()
            .map(|script| script.received.clone())
            .unwrap_or_default()
    }

    /// Makes `find` fail with `error` after recording the query.
    pub fn with_find_error(self, error: CoreError) -> Self {
        self.find
            .lock()
            .expect("fresh mutex cannot be poisoned")
            .error = Some(error);
        self
    }

    fn find_script(&self) -> Result<MutexGuard<'_, FindScript>> {
        self.find.lock().map_err(|_| poisoned())
    }

    /// Records the query and refuses a hint that names no index of `ns`, as the server does
    /// (a key-pattern hint is accepted).
    pub(super) fn note_find(&self, op: FindOperation, ns: &Namespace, q: &Query) -> Result<()> {
        self.find_script()?.received.push((op, q.clone()));
        if let Some(HintOption::Name(name)) = &q.options.hint {
            let indexes = self.indexes.lock().map_err(|_| poisoned())?;
            if !idx::list(&indexes, &key(ns))
                .iter()
                .any(|listed| listed.spec.name == *name)
            {
                return Err(CoreError::HintNotFound);
            }
        }
        Ok(())
    }

    /// The scripted failure of `find`, if one is set.
    pub(super) fn scripted_find_error(&self) -> Result<Option<CoreError>> {
        Ok(self.find_script()?.error.clone())
    }
}

/// Evaluates a query over `docs`. Fetches `limit + 1` rows to compute
/// `has_more`; `limit == 0` means "no limit", as in MongoDB.
pub(super) fn select(docs: &[Document], q: &Query) -> Result<Page> {
    check_filter(&q.filter)?;
    let sort_keys = q.sort.as_ref().map(parse_sort).transpose()?;
    let mut hits: Vec<&Document> = docs.iter().filter(|d| matches(d, &q.filter)).collect();
    if let Some(keys) = &sort_keys {
        hits.sort_by(|a, b| compare(a, b, keys));
    }
    let window = hits.into_iter().skip(q.skip as usize);
    let (fetched, limit): (Vec<&Document>, usize) = match q.limit {
        0 => (window.collect(), usize::MAX),
        n => (window.take(n as usize + 1).collect(), n as usize),
    };
    let has_more = fetched.len() > limit;
    let docs = fetched
        .into_iter()
        .take(limit)
        .map(|d| match &q.projection {
            Some(p) => project(d, p),
            None => Ok(d.clone()),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Page {
        docs,
        skip: q.skip,
        has_more,
    })
}

#[cfg(test)]
#[path = "memory_find_tests.rs"]
mod tests;
