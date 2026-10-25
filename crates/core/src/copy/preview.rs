//! Counts what a copy would do (new, changed, unchanged) by reusing the compare engine.

use std::sync::Arc;

use bson::{Bson, Document};

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

/// Compares the source (under `job.filter`) with the target, keyed on `_id`: the whole target, or
/// only the listed ids when the filter is an `_id` list (see `target_filter`). Uses the compare
/// engine's default rules for an `_id` key: strict BSON equality except field order, so a numeric
/// type difference counts as changed. `changed` therefore means "differs", whatever the conflict
/// policy: it is not "would be written" (under Skip nothing existing is rewritten). `on_progress`
/// receives the number of source documents scanned so far.
///
/// The engine keeps every new, changed AND target-only document in a capped result set
/// (`Limits::default`: 1,000,000 entries and 256 MiB). Target-only documents are ignored by the
/// counts but still consume that cap, so a preview of a small or filtered source against a large
/// existing target can fail too. Exceeding the cap fails with `CoreError::CompareCapExceeded` and
/// yields no counts.
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
        target_filter: target_filter(&job.filter),
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

/// The part of the target a preview needs. A selection of documents is an `_id` list
/// (`{_id: {$in: [..]}}`): only those ids can be new, changed or unchanged, so the target scan
/// is restricted to them instead of reading the whole collection. Any other filter keeps the
/// whole target, because which target documents match it is not known.
fn target_filter(source_filter: &Document) -> Document {
    match is_id_list(source_filter) {
        true => source_filter.clone(),
        false => Document::new(),
    }
}

fn is_id_list(filter: &Document) -> bool {
    let Some(Bson::Document(operator)) = filter.get("_id") else {
        return false;
    };
    filter.len() == 1 && operator.len() == 1 && matches!(operator.get("$in"), Some(Bson::Array(_)))
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
