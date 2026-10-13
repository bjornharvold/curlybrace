//! Applies chosen operations to the destination of a compare pair. Every operation is
//! re-verified first: both documents are re-read (with the pair's own filters), their
//! fingerprints are recomputed with the pair's rules, and both must equal what the compare
//! recorded. Anything else is skipped as stale and never written. Writes go out in batches of
//! `APPLY_BATCH`, one `bulk_write` per destination side per batch.
//!
//! A window remains between the re-read and the write (documented in the spec): a document
//! changed in that gap is overwritten. Keys and documents are data and are never logged.

use std::sync::Arc;

use bson::Document;

use super::apply::{
    APPLY_BATCH, ApplyOp, ApplyReport, Direction, IdPolicy, OpOutcome, SkipReason, kind_fits,
    map_write_failure, plan_write,
};
use super::engine::CancelFlag;
use super::fingerprint::{Fingerprint, fingerprint_of};
use super::key::KeyTuple;
use super::protocol::PairSpec;
use super::results::{Category, DiffEntry, ResultSet};
use super::rules::CompareRules;
use super::scan::fetch_one;
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace, WriteOp, WriteOutcome};

pub struct ApplyInput<'a> {
    pub spec: &'a PairSpec,
    pub results: &'a ResultSet,
    pub source: Arc<dyn DocumentStore>,
    pub target: Arc<dyn DocumentStore>,
    /// All for `spec.id`, already de-duplicated.
    pub ops: &'a [ApplyOp],
    pub policy: IdPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Source,
    Target,
}

/// What verification decided for one op.
enum Verdict {
    Done(OpOutcome),
    Write(Side, WriteOp),
}

/// One batch after verification: ops already decided, and the writes per destination side
/// (each with the index of its op within the batch).
#[derive(Default)]
struct Planned {
    done: Vec<Option<OpOutcome>>,
    to_source: Vec<(usize, WriteOp)>,
    to_target: Vec<(usize, WriteOp)>,
}

const CATEGORIES: [Category; 3] = [
    Category::OnlyInSource,
    Category::OnlyInTarget,
    Category::Different,
];

fn internal_error() -> CoreError {
    CoreError::Database("internal error".into())
}

impl ApplyInput<'_> {
    fn store(&self, side: Side) -> &Arc<dyn DocumentStore> {
        match side {
            Side::Source => &self.source,
            Side::Target => &self.target,
        }
    }

    fn namespace(&self, side: Side) -> &Namespace {
        match side {
            Side::Source => &self.spec.source,
            Side::Target => &self.spec.target,
        }
    }

    fn filter(&self, side: Side) -> &Document {
        match side {
            Side::Source => &self.spec.source_filter,
            Side::Target => &self.spec.target_filter,
        }
    }

    /// The document with `key` on `side`, as the pair's filter sees it.
    async fn read(&self, side: Side, key: &KeyTuple) -> Result<Option<Document>> {
        fetch_one(
            self.store(side),
            self.namespace(side),
            self.filter(side),
            &self.spec.key,
            key,
        )
        .await
    }

    fn recorded(&self, op: &ApplyOp) -> Option<&DiffEntry> {
        if op.pair != self.spec.id {
            return None;
        }
        CATEGORIES
            .iter()
            .find_map(|c| self.results.find(*c, &op.key))
    }
}

/// Does the freshly read document still match what the compare recorded (`None` = absent)?
fn unchanged(
    rules: &CompareRules,
    recorded: Option<Fingerprint>,
    doc: Option<&Document>,
) -> Result<bool> {
    Ok(match (recorded, doc) {
        (None, None) => true,
        (Some(expected), Some(doc)) => fingerprint_of(doc, rules)? == expected,
        _ => false,
    })
}

async fn verify(input: &ApplyInput<'_>, op: &ApplyOp) -> Result<Verdict> {
    let entry = input.recorded(op).ok_or(CoreError::NotFound)?;
    let (has_source, has_target) = (entry.source.is_some(), entry.target.is_some());
    if !kind_fits(op.kind, op.direction, has_source, has_target) {
        return Ok(Verdict::Done(OpOutcome::Skipped(SkipReason::NotPlanned)));
    }
    let (source, target) = tokio::try_join!(
        input.read(Side::Source, &op.key),
        input.read(Side::Target, &op.key)
    )?;
    let rules = &input.spec.rules;
    if !unchanged(rules, entry.source, source.as_ref())?
        || !unchanged(rules, entry.target, target.as_ref())?
    {
        return Ok(Verdict::Done(OpOutcome::Skipped(SkipReason::Stale)));
    }
    let (origin, dest, side) = match op.direction {
        Direction::SourceToTarget => (&source, &target, Side::Target),
        Direction::TargetToSource => (&target, &source, Side::Source),
    };
    let write = plan_write(op.kind, origin.as_ref(), dest.as_ref(), input.policy)?;
    Ok(Verdict::Write(side, write))
}

/// Verifies one op; an error becomes that op's failure and nothing is written for it.
async fn verify_op(input: &ApplyInput<'_>, op: &ApplyOp) -> Verdict {
    match verify(input, op).await {
        Ok(verdict) => verdict,
        Err(error) => Verdict::Done(OpOutcome::Failed(error)),
    }
}

async fn plan_batch(input: &ApplyInput<'_>, batch: &[ApplyOp]) -> Planned {
    let mut planned = Planned::default();
    for (index, op) in batch.iter().enumerate() {
        match verify_op(input, op).await {
            Verdict::Done(outcome) => planned.done.push(Some(outcome)),
            Verdict::Write(side, write) => {
                planned.done.push(None);
                match side {
                    Side::Source => planned.to_source.push((index, write)),
                    Side::Target => planned.to_target.push((index, write)),
                }
            }
        }
    }
    planned
}

fn to_outcome(outcome: WriteOutcome) -> OpOutcome {
    outcome.map_or_else(
        |f| OpOutcome::Failed(map_write_failure(f)),
        |()| OpOutcome::Applied,
    )
}

/// One `bulk_write` for `group`; a whole-call error fails every op of the group.
async fn write_group(
    input: &ApplyInput<'_>,
    side: Side,
    group: Vec<(usize, WriteOp)>,
) -> Vec<(usize, OpOutcome)> {
    if group.is_empty() {
        return Vec::new();
    }
    let (indexes, writes): (Vec<usize>, Vec<WriteOp>) = group.into_iter().unzip();
    let result = input
        .store(side)
        .bulk_write(input.namespace(side), writes)
        .await;
    match result {
        Ok(outcomes) if outcomes.len() == indexes.len() => indexes
            .into_iter()
            .zip(outcomes.into_iter().map(to_outcome))
            .collect(),
        Ok(_) => fail_all(indexes, &internal_error()),
        Err(error) => fail_all(indexes, &error),
    }
}

fn fail_all(indexes: Vec<usize>, error: &CoreError) -> Vec<(usize, OpOutcome)> {
    indexes
        .into_iter()
        .map(|i| (i, OpOutcome::Failed(error.clone())))
        .collect()
}

/// The final outcome of every op of the batch, in order. An op that was neither decided nor
/// written (cannot happen) is reported failed rather than applied.
fn merge_outcomes(
    done: Vec<Option<OpOutcome>>,
    written: Vec<(usize, OpOutcome)>,
) -> Vec<OpOutcome> {
    let mut slots = done;
    for (index, outcome) in written {
        if let Some(slot) = slots.get_mut(index) {
            *slot = Some(outcome);
        }
    }
    slots
        .into_iter()
        .map(|o| o.unwrap_or_else(|| OpOutcome::Failed(internal_error())))
        .collect()
}

async fn apply_batch(input: &ApplyInput<'_>, batch: &[ApplyOp]) -> Vec<OpOutcome> {
    let Planned {
        done,
        to_source,
        to_target,
    } = plan_batch(input, batch).await;
    let (mut written, to_target_done) = tokio::join!(
        write_group(input, Side::Source, to_source),
        write_group(input, Side::Target, to_target)
    );
    written.extend(to_target_done);
    merge_outcomes(done, written)
}

/// Verifies and applies `ops` in batches of `APPLY_BATCH`; `progress(done_in_pair)` after each
/// batch. Cancel is checked before each batch; the ops left are counted `not_run`.
pub async fn apply_pair(
    input: ApplyInput<'_>,
    cancel: &CancelFlag,
    progress: &mut dyn FnMut(u64),
) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    let mut done = 0usize;
    for batch in input.ops.chunks(APPLY_BATCH) {
        if cancel.is_cancelled() {
            break;
        }
        let outcomes = apply_batch(&input, batch).await;
        for (op, outcome) in batch.iter().zip(&outcomes) {
            report.record(&op.key, outcome);
        }
        done += batch.len();
        progress(done as u64);
    }
    report.not_run = (input.ops.len() - done) as u64;
    Ok(report)
}

#[cfg(test)]
#[path = "apply_engine_tests.rs"]
mod tests;
