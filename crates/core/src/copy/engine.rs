//! Streams one collection from a source store into a target store under a conflict policy.
//!
//! Documents are read in ascending `_id` order (the compare machinery's BSON order, whatever
//! sort the UI shows), one batch at a time: the batch is planned against the `_id`s the target
//! already holds, then written with one `bulk_write`. Memory is bounded by `CopyJob::batch`.
//! Nothing is ever deleted from the target.
//!
//! Cancellation is observed between batches and during reads, which are abandoned mid-flight.
//! The `bulk_write` of a batch is never abandoned, so the counts are exact: `written` is the
//! number of documents the target acknowledged, and a batch that has not reached `bulk_write`
//! when the cancel arrives contributes nothing. (A caller that drops the whole future instead of
//! cancelling the flag loses that guarantee for the batch in flight.)
//!
//! Document values and `_id` values are never logged or put in an error. `CopyFailure::key` is
//! returned for the UI to display and is bounded to `MAX_REPORTED_FAILURES`; more failures are
//! only counted.

use std::sync::Arc;

use bson::{Bson, Document, doc};

use super::names::is_same_namespace;
use super::policy::{ConflictPolicy, ExistingIds, plan_batch};
use super::structure::{IndexReport, StructureReport};
use crate::compare::apply::MAX_REPORTED_FAILURES;
use crate::compare::engine::CancelFlag;
use crate::compare::key::{KeySpec, KeyTuple, compare_tuples, extract_key};
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace, Query, WriteFailure, WriteOp, WriteOutcome};

/// One collection to copy.
#[derive(Clone)]
pub struct CopyJob {
    pub source: Arc<dyn DocumentStore>,
    pub source_ns: Namespace,
    /// Which source documents to copy; empty means the whole collection.
    pub filter: Document,
    pub target: Arc<dyn DocumentStore>,
    pub target_ns: Namespace,
    pub policy: ConflictPolicy,
    /// Documents per batch (`APPLY_BATCH` in production; tests lower it).
    pub batch: usize,
}

/// Running totals. `scanned` counts documents read from the source, including those of a batch
/// whose write then failed as a whole.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyProgress {
    pub scanned: u64,
    pub written: u64,
    pub skipped: u64,
    pub failed: u64,
}

/// One document the target refused. The key is for the UI only and is never logged.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyFailure {
    pub key: Bson,
    pub reason: WriteFailure,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CopyOutcome {
    Completed,
    Cancelled,
    /// The copy stopped: a read failed, or a `bulk_write` failed as a whole. With options and
    /// indexes requested it is also the outcome when the target could not be created, or when the
    /// indexes could not be listed after the data was copied (the counts stay exact then).
    Failed(CoreError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopyReport {
    pub progress: CopyProgress,
    /// The first `MAX_REPORTED_FAILURES` failures; the rest are only in `progress.failed`.
    pub failures: Vec<CopyFailure>,
    pub outcome: CopyOutcome,
    /// What happened to the target collection's options; `None` when options were not copied.
    pub structure: Option<StructureReport>,
    /// The indexes built after the data; `None` when they were not copied, or when the data copy
    /// was cancelled or failed (no index is built then).
    pub indexes: Option<IndexReport>,
}

/// One batch read from the source and planned against the target, not yet written.
struct Prepared {
    scanned: u64,
    skipped: u64,
    ops: Vec<WriteOp>,
    /// Where the next read starts; `None` when the source is exhausted.
    resume: Option<KeyTuple>,
}

/// Copies the collection. Calls `on_progress` after every written batch.
pub async fn copy_collection(
    job: CopyJob,
    cancel: &CancelFlag,
    on_progress: &mut (dyn FnMut(CopyProgress) + Send),
) -> CopyReport {
    let mut report = CopyReport {
        progress: CopyProgress::default(),
        failures: Vec::new(),
        outcome: CopyOutcome::Completed,
        structure: None,
        indexes: None,
    };
    if is_same_collection(&job) {
        report.outcome = CopyOutcome::Failed(CoreError::Unsupported(
            "a collection cannot be copied onto itself".into(),
        ));
        return report;
    }
    report.outcome = stream(&job, cancel, on_progress, &mut report).await;
    report
}

/// The same collection of the same store. Stores are told apart by the identity of their `Arc`
/// (a `Namespace` alone cannot); two wrappers around one store look different and are allowed.
pub(super) fn is_same_collection(job: &CopyJob) -> bool {
    let same_store = std::ptr::addr_eq(Arc::as_ptr(&job.source), Arc::as_ptr(&job.target));
    same_store && is_same_namespace(&job.source_ns, &job.target_ns)
}

async fn stream(
    job: &CopyJob,
    cancel: &CancelFlag,
    on_progress: &mut (dyn FnMut(CopyProgress) + Send),
    report: &mut CopyReport,
) -> CopyOutcome {
    let mut from: Option<KeyTuple> = None;
    loop {
        if cancel.is_cancelled() {
            return CopyOutcome::Cancelled;
        }
        let prepared = match cancel
            .run_until_cancelled(prepare(job, from.as_ref()))
            .await
        {
            None => return CopyOutcome::Cancelled,
            Some(Err(err)) => return CopyOutcome::Failed(err),
            Some(Ok(prepared)) => prepared,
        };
        let Prepared {
            scanned,
            skipped,
            ops,
            resume,
        } = prepared;
        report.progress.scanned += scanned;
        report.progress.skipped += skipped;
        if let Err(err) = write(job, ops, report).await {
            return CopyOutcome::Failed(err);
        }
        on_progress(report.progress);
        match resume {
            Some(next) => from = Some(next),
            None => return CopyOutcome::Completed,
        }
    }
}

/// Reads the next batch and plans its writes.
async fn prepare(job: &CopyJob, from: Option<&KeyTuple>) -> Result<Prepared> {
    let key = KeySpec::id();
    let batch = job.batch.max(1);
    // The scan includes the start key, so one extra document is asked for when resuming.
    let wanted = batch + usize::from(from.is_some());
    let limit = u32::try_from(wanted).unwrap_or(u32::MAX);
    let fetched = job
        .source
        .scan_sorted(&job.source_ns, &job.filter, &key, from, limit)
        .await?;
    let may_have_more = fetched.len() >= wanted;
    let docs = drop_resumed(fetched, &key, from)?;
    let resume = match (may_have_more, docs.last()) {
        (true, Some(last)) => Some(extract_key(last, &key)?),
        _ => None,
    };
    let scanned = docs.len() as u64;
    let existing = existing_ids(job, &docs).await?;
    let plan = plan_batch(job.policy, docs, &existing);
    Ok(Prepared {
        scanned,
        skipped: plan.skipped,
        ops: plan.ops,
        resume,
    })
}

/// Drops the document equal to `from`, which the previous batch already processed.
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

/// Which of the batch's `_id`s the target already holds.
async fn existing_ids(job: &CopyJob, docs: &[Document]) -> Result<ExistingIds> {
    let ids: Vec<Bson> = docs.iter().filter_map(|d| d.get("_id").cloned()).collect();
    if ids.is_empty() {
        return Ok(ExistingIds::default());
    }
    let query = Query {
        filter: doc! { "_id": { "$in": ids } },
        projection: Some(doc! { "_id": 1 }),
        limit: 0,
        ..Query::default()
    };
    let page = job.target.find(&job.target_ns, &query).await?;
    let found = page
        .docs
        .into_iter()
        .filter_map(|d| d.get("_id").cloned())
        .collect();
    Ok(ExistingIds::from_ids(found))
}

/// Writes one batch and folds its outcomes into `report`. `Err` when the call failed as a whole.
async fn write(job: &CopyJob, ops: Vec<WriteOp>, report: &mut CopyReport) -> Result<()> {
    if ops.is_empty() {
        return Ok(());
    }
    let keys: Vec<Bson> = ops.iter().map(op_key).collect();
    let outcomes = job.target.bulk_write(&job.target_ns, ops).await?;
    if outcomes.len() != keys.len() {
        return Err(CoreError::ApplyOutcomeUnknown);
    }
    for (key, outcome) in keys.into_iter().zip(outcomes) {
        record(report, key, outcome);
    }
    Ok(())
}

fn record(report: &mut CopyReport, key: Bson, outcome: WriteOutcome) {
    match outcome {
        Ok(()) => report.progress.written += 1,
        Err(reason) => {
            report.progress.failed += 1;
            if report.failures.len() < MAX_REPORTED_FAILURES {
                report.failures.push(CopyFailure { key, reason });
            }
        }
    }
}

fn op_key(op: &WriteOp) -> Bson {
    match op {
        WriteOp::Insert { doc } => doc.get("_id").cloned().unwrap_or(Bson::Null),
        WriteOp::Replace { id, .. } | WriteOp::Delete { id } | WriteOp::UpdateFields { id, .. } => {
            id.clone()
        }
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
