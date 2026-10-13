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
    APPLY_BATCH, ApplyKind, ApplyOp, ApplyReport, Direction, IdPolicy, OpOutcome, SkipReason,
    kind_fits, map_write_failure, plan_write,
};
use super::apply_fields::plan_fields;
use super::diff::diff_normalized;
use super::engine::CancelFlag;
use super::fingerprint::{Fingerprint, fingerprint_of};
use super::key::{KeyTuple, compare_tuples};
use super::normalize::normalize;
use super::protocol::PairSpec;
use super::results::{Category, DiffEntry, ResultSet};
use super::rules::CompareRules;
use super::scan::{Unique, fetch_unique};
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
    async fn read(&self, side: Side, key: &KeyTuple) -> Result<Unique> {
        fetch_unique(
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

fn into_option(found: Unique) -> Option<Document> {
    match found {
        Unique::One(doc) => Some(doc),
        Unique::Absent | Unique::Several => None,
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

fn not_planned() -> Verdict {
    Verdict::Done(OpOutcome::Skipped(SkipReason::NotPlanned))
}

type Oriented<'a> = (&'a Option<Document>, &'a Option<Document>, Side);

/// `(origin, destination, destination side)` of a direction.
fn orient<'a>(
    direction: Direction,
    source: &'a Option<Document>,
    target: &'a Option<Document>,
) -> Oriented<'a> {
    match direction {
        Direction::SourceToTarget => (source, target, Side::Target),
        Direction::TargetToSource => (target, source, Side::Source),
    }
}

/// The field-level write of a `Fields` op, planned from a fresh diff of the two verified
/// documents. Anything `plan_fields` refuses (not a real, addressable, scope-safe change) is
/// skipped as not planned and nothing is written for the document.
fn plan_fields_op(
    rules: &CompareRules,
    op: &ApplyOp,
    source: &Option<Document>,
    target: &Option<Document>,
) -> Result<Verdict> {
    let (Some(source_doc), Some(target_doc)) = (source, target) else {
        return Ok(not_planned());
    };
    let changes = diff_normalized(
        &normalize(source_doc, rules)?,
        &normalize(target_doc, rules)?,
        rules.ignore_array_order,
    )?;
    let (Some(origin), Some(dest), side) = orient(op.direction, source, target) else {
        return Ok(not_planned());
    };
    let planned = plan_fields(
        &op.fields,
        &changes,
        op.direction,
        origin,
        dest,
        &rules.field_scope,
    );
    Ok(planned.map_or_else(|_| not_planned(), |write| Verdict::Write(side, write)))
}

async fn verify(input: &ApplyInput<'_>, op: &ApplyOp) -> Result<Verdict> {
    let entry = input.recorded(op).ok_or(CoreError::NotFound)?;
    // Defence in depth: the runtime refuses these up front.
    if (op.kind == ApplyKind::Fields) == op.fields.is_empty() {
        return Ok(not_planned());
    }
    let (has_source, has_target) = (entry.source.is_some(), entry.target.is_some());
    if !kind_fits(op.kind, op.direction, has_source, has_target) {
        return Ok(Verdict::Done(OpOutcome::Skipped(SkipReason::NotPlanned)));
    }
    let (source, target) = tokio::try_join!(
        input.read(Side::Source, &op.key),
        input.read(Side::Target, &op.key)
    )?;
    let (Unique::Absent | Unique::One(_), Unique::Absent | Unique::One(_)) = (&source, &target)
    else {
        return Ok(Verdict::Done(OpOutcome::Skipped(SkipReason::Stale)));
    };
    let (source, target) = (into_option(source), into_option(target));
    let rules = &input.spec.rules;
    if !unchanged(rules, entry.source, source.as_ref())?
        || !unchanged(rules, entry.target, target.as_ref())?
    {
        return Ok(Verdict::Done(OpOutcome::Skipped(SkipReason::Stale)));
    }
    if op.kind == ApplyKind::Fields {
        return plan_fields_op(rules, op, &source, &target);
    }
    let (origin, dest, side) = orient(op.direction, &source, &target);
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

async fn plan_batch(input: &ApplyInput<'_>, batch: &[ApplyOp], repeated: &[bool]) -> Planned {
    let mut planned = Planned::default();
    for (index, op) in batch.iter().enumerate() {
        let verdict = if repeated[index] {
            Verdict::Done(OpOutcome::Skipped(SkipReason::NotPlanned))
        } else {
            verify_op(input, op).await
        };
        match verdict {
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

/// One `bulk_write` for `group`. A whole-call error (or a reply that does not line up with the
/// ops) fails every op of the group as `ApplyOutcomeUnknown`: earlier commands of the call may
/// have landed, and driver text is never passed on.
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
        Ok(_) | Err(_) => fail_all(indexes, &CoreError::ApplyOutcomeUnknown),
    }
}

fn fail_all(indexes: Vec<usize>, error: &CoreError) -> Vec<(usize, OpOutcome)> {
    indexes
        .into_iter()
        .map(|i| (i, OpOutcome::Failed(error.clone())))
        .collect()
}

/// The final outcome of every op of the batch, in order. An op that was neither decided nor
/// written (cannot happen) is reported `ApplyOutcomeUnknown` rather than applied.
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
        .map(|o| o.unwrap_or(OpOutcome::Failed(CoreError::ApplyOutcomeUnknown)))
        .collect()
}

async fn apply_batch(
    input: &ApplyInput<'_>,
    batch: &[ApplyOp],
    repeated: &[bool],
) -> Vec<OpOutcome> {
    let Planned {
        done,
        to_source,
        to_target,
    } = plan_batch(input, batch, repeated).await;
    let (mut written, to_target_done) = tokio::join!(
        write_group(input, Side::Source, to_source),
        write_group(input, Side::Target, to_target)
    );
    written.extend(to_target_done);
    merge_outcomes(done, written)
}

/// Flags every op whose key (under key equality) already appeared earlier in `ops`: only the
/// first op per key may run, so two ops can never act on the same document in one apply.
fn repeated_keys(ops: &[ApplyOp]) -> Vec<bool> {
    let mut order: Vec<usize> = (0..ops.len()).collect();
    // Stable: within equal keys the original order is kept, so the first stays first.
    order.sort_by(|a, b| compare_tuples(&ops[*a].key, &ops[*b].key));
    let mut flags = vec![false; ops.len()];
    for pair in order.windows(2) {
        if compare_tuples(&ops[pair[0]].key, &ops[pair[1]].key).is_eq() {
            flags[pair[1]] = true;
        }
    }
    flags
}

/// Verifies and applies `ops` in batches of `APPLY_BATCH`; `progress(done_in_pair)` after each
/// batch. Cancel is checked before each batch; the ops left are counted `not_run`.
pub async fn apply_pair(
    input: ApplyInput<'_>,
    cancel: &CancelFlag,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    let mut done = 0usize;
    let repeated = repeated_keys(input.ops);
    for (batch, flags) in input
        .ops
        .chunks(APPLY_BATCH)
        .zip(repeated.chunks(APPLY_BATCH))
    {
        if cancel.is_cancelled() {
            break;
        }
        let outcomes = apply_batch(&input, batch, flags).await;
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

#[cfg(test)]
#[path = "apply_fields_engine_tests.rs"]
mod fields_tests;
