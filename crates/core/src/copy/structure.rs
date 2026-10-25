//! Collection options first, data next, indexes last.
//!
//! With the paste dialog's "also copy indexes and collection options" checked:
//! - the target collection is created with the source's options, only when it does not exist yet
//!   on the target. An existing target is never touched: its options stay as they are;
//! - the data is copied next, exactly as without the option;
//! - indexes are built last, and only when the data copy completed (a cancelled or failed copy
//!   builds none). Only indexes the target lacks by name are built; the `_id_` index is never
//!   copied; an index that fails to build is reported (name and typed reason) and the copied
//!   data stays.
//!
//! Views and time-series collections are not supported: their data is copied as it would be
//! without the option, and the report says so (`StructureOutcome::SourceNotPlain`).
//!
//! Index names are metadata and appear in the report; document values, `_id`s and server error
//! text never do (an index build error can quote a duplicate value, so it is reduced to
//! `IndexFailReason`).

use std::collections::HashSet;

use super::engine::{
    CopyJob, CopyOutcome, CopyProgress, CopyReport, copy_collection, is_same_collection,
};
use crate::compare::engine::CancelFlag;
use crate::error::{CoreError, Result};
use crate::indexes::IndexSpec;
use crate::store::CollectionKind;

/// The name of the index every collection has; it is never copied.
const ID_INDEX: &str = "_id_";

/// What became of the target collection's options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureOutcome {
    /// The target did not exist and was created with the source's options.
    Created,
    /// The target already existed: its options were left as they are.
    ExistingKept,
    /// The source is a view or a time-series collection: data only, no options, no indexes.
    SourceNotPlain,
    /// The source collection does not exist: nothing to take options or indexes from.
    SourceMissing,
}

impl StructureOutcome {
    /// Whether the indexes of the source are copied after the data.
    fn copies_indexes(self) -> bool {
        matches!(self, Self::Created | Self::ExistingKept)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructureReport {
    pub outcome: StructureOutcome,
}

/// Why one index could not be built. Never carries server text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexFailReason {
    /// A unique index over data that holds duplicates.
    DuplicateData,
    /// The target cannot build this kind of index.
    Unsupported,
    /// The connection may not create indexes.
    NotAllowed,
    /// The connection was lost while the index was built.
    ConnectionLost,
    /// The server refused the definition for another reason.
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexFailure {
    pub name: String,
    pub reason: IndexFailReason,
}

/// What happened to each non-`_id` index of the source, by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexReport {
    /// Built on the target.
    pub built: Vec<String>,
    /// The target already had an index of this name: left alone.
    pub existing: Vec<String>,
    pub failed: Vec<IndexFailure>,
}

/// Creates the target with the source's options when the target does not exist yet.
pub async fn copy_structure_before(job: &CopyJob) -> Result<StructureReport> {
    let outcome = structure_outcome(job).await?;
    Ok(StructureReport { outcome })
}

async fn structure_outcome(job: &CopyJob) -> Result<StructureOutcome> {
    if !source_is_plain(job).await? {
        return Ok(StructureOutcome::SourceNotPlain);
    }
    let Some(options) = job.source.collection_options(&job.source_ns).await? else {
        return Ok(StructureOutcome::SourceMissing);
    };
    // Check-then-create is racy by nature; a collection that appears in between makes
    // `create_collection` fail, and the item then fails with that typed error.
    if job
        .target
        .collection_options(&job.target_ns)
        .await?
        .is_some()
    {
        return Ok(StructureOutcome::ExistingKept);
    }
    job.target
        .create_collection(&job.target_ns, &options)
        .await?;
    Ok(StructureOutcome::Created)
}

/// False for a view or a time-series collection. A source missing from the listing is "plain":
/// there is nothing to read from it either way.
async fn source_is_plain(job: &CopyJob) -> Result<bool> {
    let kinds = job.source.list_collection_kinds(&job.source_ns.db).await?;
    let kind = kinds
        .into_iter()
        .find(|(name, _)| *name == job.source_ns.collection)
        .map(|(_, kind)| kind);
    Ok(matches!(kind, None | Some(CollectionKind::Collection)))
}

/// Builds the indexes the target lacks by name. A build that fails is recorded and the next one
/// is tried; `Err` is only for failing to list the indexes.
pub async fn copy_indexes_after(job: &CopyJob) -> Result<IndexReport> {
    let wanted = specs_of(job, true).await?;
    let present: HashSet<String> = specs_of(job, false)
        .await?
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    let mut report = IndexReport::default();
    for spec in wanted.into_iter().filter(|spec| spec.name != ID_INDEX) {
        report = build_one(job, spec, &present, report).await;
    }
    Ok(report)
}

/// The index definitions of the source or the target; a missing collection has none.
async fn specs_of(job: &CopyJob, source: bool) -> Result<Vec<IndexSpec>> {
    let (store, ns) = match source {
        true => (&job.source, &job.source_ns),
        false => (&job.target, &job.target_ns),
    };
    match store.list_index_specs(ns).await {
        Err(CoreError::NotFound) => Ok(Vec::new()),
        other => other,
    }
}

async fn build_one(
    job: &CopyJob,
    spec: IndexSpec,
    present: &HashSet<String>,
    report: IndexReport,
) -> IndexReport {
    let name = spec.name.clone();
    if present.contains(&name) {
        return IndexReport {
            existing: [report.existing, vec![name]].concat(),
            ..report
        };
    }
    match job.target.create_index(&job.target_ns, &spec).await {
        Ok(()) => IndexReport {
            built: [report.built, vec![name]].concat(),
            ..report
        },
        Err(error) => {
            let failure = IndexFailure {
                name,
                reason: classify_index_error(&error),
            };
            IndexReport {
                failed: [report.failed, vec![failure]].concat(),
                ..report
            }
        }
    }
}

/// Reduces a store error to a reason that is safe to show: the text is dropped (and not logged,
/// since a duplicate-key message quotes a value).
fn classify_index_error(error: &CoreError) -> IndexFailReason {
    match error {
        CoreError::Database(text) if text.contains("E11000") || text.contains("duplicate key") => {
            IndexFailReason::DuplicateData
        }
        CoreError::Unsupported(_) => IndexFailReason::Unsupported,
        CoreError::Unauthorized | CoreError::AuthFailed | CoreError::ReadOnly => {
            IndexFailReason::NotAllowed
        }
        CoreError::Unreachable
        | CoreError::UnreachableSsh
        | CoreError::ConnectionInterrupted(_)
        | CoreError::WriteOutcomeUnknown => IndexFailReason::ConnectionLost,
        _ => IndexFailReason::Rejected,
    }
}

/// Copies one collection. With `with_structure` off this is exactly [`copy_collection`]. With it
/// on, see the module docs for the order and for when no index is built.
pub async fn copy_with_structure(
    job: CopyJob,
    with_structure: bool,
    cancel: &CancelFlag,
    on_progress: &mut (dyn FnMut(CopyProgress) + Send),
) -> CopyReport {
    // The engine answers a cancel and a copy onto itself without any write; nothing is created
    // before it has had the chance to.
    if !with_structure || cancel.is_cancelled() || is_same_collection(&job) {
        return copy_collection(job, cancel, on_progress).await;
    }
    let structure = match copy_structure_before(&job).await {
        Ok(structure) => structure,
        Err(error) => return structure_failed(error),
    };
    let mut report = copy_collection(job.clone(), cancel, on_progress).await;
    report.structure = Some(structure);
    let wanted = report.outcome == CopyOutcome::Completed
        && structure.outcome.copies_indexes()
        && !cancel.is_cancelled();
    if !wanted {
        return report;
    }
    match copy_indexes_after(&job).await {
        Ok(indexes) => CopyReport {
            indexes: Some(indexes),
            ..report
        },
        Err(error) => CopyReport {
            outcome: CopyOutcome::Failed(error),
            ..report
        },
    }
}

/// The options could not be set up: nothing was copied.
fn structure_failed(error: CoreError) -> CopyReport {
    CopyReport {
        progress: CopyProgress::default(),
        failures: Vec::new(),
        outcome: CopyOutcome::Failed(error),
        structure: None,
        indexes: None,
    }
}

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;
