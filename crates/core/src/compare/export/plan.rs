//! Export planning: the apply planner's verdicts, collected instead of written.
//!
//! `plan_pair_export` runs EXACTLY the verification of an apply (`apply_engine::plan_batch`:
//! re-read both documents with the pair's filters, compare fingerprints with the recorded
//! entry, plan the write for the chosen direction) and hands the result to a sink. It never
//! writes: the stores are only read. The one extra step is a dry render of each planned write,
//! so a write the shell cannot represent becomes `Skipped(Unsupported)` BEFORE any writer sees
//! it and the script and the JSON plan always list the same operations.

use super::ExportSkip;
use super::mongosh::{ShellOp, check_op};
use crate::compare::apply::{APPLY_BATCH, ApplyOp, Direction, OpOutcome};
use crate::compare::apply_engine::{ApplyInput, PlannedOp, Verdict, plan_batch, repeated_keys};
use crate::compare::engine::CancelFlag;
use crate::compare::fingerprint::Fingerprint;
use crate::compare::protocol::Side;
use crate::error::{CoreError, Result};
use crate::store::WriteOp;

/// One op of the plan, in op order. Fingerprints are the ones the compare recorded.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportItem {
    /// The write an apply would do now, against the destination side of the op's direction.
    Write {
        op: ApplyOp,
        write: WriteOp,
        source_fp: Option<Fingerprint>,
        target_fp: Option<Fingerprint>,
    },
    Skipped {
        op: ApplyOp,
        reason: ExportSkip,
        source_fp: Option<Fingerprint>,
        target_fp: Option<Fingerprint>,
    },
    /// A read error while verifying; value-free.
    Failed { op: ApplyOp, error: CoreError },
}

/// The side an op writes to, derived from its direction in this one place (the same destination
/// apply writes to).
pub fn destination_side(op: &ApplyOp) -> Side {
    match op.direction {
        Direction::SourceToTarget => Side::Target,
        Direction::TargetToSource => Side::Source,
    }
}

impl ExportItem {
    /// The op this item is about.
    pub fn op(&self) -> &ApplyOp {
        match self {
            Self::Write { op, .. } | Self::Skipped { op, .. } | Self::Failed { op, .. } => op,
        }
    }

    /// The side a `Write` goes to ([`destination_side`]); `None` for items that are not writes.
    pub fn destination(&self) -> Option<Side> {
        match self {
            Self::Write { op, .. } => Some(destination_side(op)),
            _ => None,
        }
    }
}

/// What happened to the ops requested; the fields sum to `ops.len()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportPlanCounts {
    pub written: u64,
    pub skipped_stale: u64,
    pub skipped_not_planned: u64,
    pub skipped_unsupported: u64,
    pub failed: u64,
    pub not_run: u64,
}

impl ExportPlanCounts {
    fn record(&mut self, item: &ExportItem) {
        match item {
            ExportItem::Write { .. } => self.written += 1,
            ExportItem::Skipped { reason, .. } => match reason {
                ExportSkip::Stale => self.skipped_stale += 1,
                ExportSkip::NotPlanned => self.skipped_not_planned += 1,
                ExportSkip::Unsupported => self.skipped_unsupported += 1,
            },
            ExportItem::Failed { .. } => self.failed += 1,
        }
    }
}

/// The item for one planned op. A write the shell cannot represent (the dry render fails; its
/// error is dropped, it only ever says "cannot render") is skipped here, so no writer sees it.
fn item_of(op: &ApplyOp, planned: PlannedOp) -> ExportItem {
    let PlannedOp {
        verdict,
        source_fp,
        target_fp,
    } = planned;
    let op = op.clone();
    match verdict {
        Verdict::Write(_, write) if check_op(&ShellOp::from(write.clone())).is_ok() => {
            ExportItem::Write {
                op,
                write,
                source_fp,
                target_fp,
            }
        }
        Verdict::Write(..) => skipped(op, ExportSkip::Unsupported, source_fp, target_fp),
        Verdict::Done(OpOutcome::Skipped(reason)) => {
            skipped(op, reason.into(), source_fp, target_fp)
        }
        Verdict::Done(OpOutcome::Failed(error)) => ExportItem::Failed { op, error },
        // Planning never applies; treated like apply's "neither decided nor written".
        Verdict::Done(OpOutcome::Applied) => ExportItem::Failed {
            op,
            error: CoreError::ApplyOutcomeUnknown,
        },
    }
}

fn skipped(
    op: ApplyOp,
    reason: ExportSkip,
    source_fp: Option<Fingerprint>,
    target_fp: Option<Fingerprint>,
) -> ExportItem {
    ExportItem::Skipped {
        op,
        reason,
        source_fp,
        target_fp,
    }
}

/// Plans (verifies, never writes) the ops of ONE pair in batches of `APPLY_BATCH`. `sink`
/// receives each batch's items in op order; `progress(done_in_pair)` follows each batch. Cancel
/// is checked before each batch; the ops left are counted `not_run`. A sink error stops the plan.
pub async fn plan_pair_export(
    input: ApplyInput<'_>,
    cancel: &CancelFlag,
    sink: &mut (dyn FnMut(Vec<ExportItem>) -> Result<()> + Send),
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<ExportPlanCounts> {
    let mut counts = ExportPlanCounts::default();
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
        let planned = plan_batch(&input, batch, flags).await;
        let items: Vec<ExportItem> = batch
            .iter()
            .zip(planned)
            .map(|(op, planned)| item_of(op, planned))
            .collect();
        items.iter().for_each(|item| counts.record(item));
        sink(items)?;
        done += batch.len();
        progress(done as u64);
    }
    counts.not_run = (input.ops.len() - done) as u64;
    Ok(counts)
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
