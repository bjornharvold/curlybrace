//! Counts what a copy would do (new, changed, unchanged) by reusing the compare engine.

use std::sync::Arc;

use bson::Document;

use crate::compare::engine::{CancelFlag, EngineOutcome, PairInput, run_pair};
use crate::compare::key::KeySpec;
use crate::compare::results::{Budget, Limits};
use crate::compare::rules::CompareRules;
use crate::error::Result;

use super::engine::CopyJob;

/// What a copy would do with the source documents. Target-only documents are not counted: a copy
/// never deletes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreviewCounts {
    pub new: u64,
    pub changed: u64,
    pub unchanged: u64,
}

/// Compares the source (under `job.filter`) with the whole target, keyed on `_id`. Uses the compare
/// engine's default rules for an `_id` key: strict BSON equality except field order, so a numeric
/// type difference counts as changed. `on_progress` receives the number of source documents
/// scanned so far.
///
/// The engine keeps every new and changed document in a capped result set (the default `Limits`);
/// a preview that exceeds the cap fails with `CoreError::CompareCapExceeded` and yields no counts.
pub async fn preview_collection(
    job: &CopyJob,
    cancel: &CancelFlag,
    on_progress: &mut (dyn FnMut(u64) + Send),
) -> Result<(EngineOutcome, PreviewCounts)> {
    let key = KeySpec::id();
    let input = PairInput {
        source: Arc::clone(&job.source),
        source_ns: job.source_ns.clone(),
        source_filter: job.filter.clone(),
        target: Arc::clone(&job.target),
        target_ns: job.target_ns.clone(),
        target_filter: Document::new(),
        rules: CompareRules::for_key(&key),
        key,
        budget: Budget::new(Limits::default()),
    };
    let (outcome, summary, _) =
        run_pair(input, cancel, &mut |p| on_progress(p.scanned_source)).await?;
    let counts = PreviewCounts {
        new: summary.only_in_source,
        changed: summary.different,
        unchanged: summary.equal,
    };
    Ok((outcome, counts))
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
