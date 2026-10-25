//! The counting loop of a `Preview`: one collection after another through the compare engine.

use super::{CopyEvent, CopyItem, Job};
use crate::compare::engine::EngineOutcome;
use crate::copy::{PreviewCounts, preview_collection};
use crate::error::{CoreError, Result};

/// Counts every item; returns the terminal event. Counts that cannot be had because the compare
/// result cap was exceeded are `None`, never a failure; any other error ends the run. A cancel
/// ends it with `Done { cancelled: true }`.
pub(super) async fn run(job: &Job, items: &[CopyItem]) -> CopyEvent {
    let run = job.ctx.run;
    let cancelled = CopyEvent::Done {
        run,
        cancelled: true,
    };
    for (index, item) in items.iter().enumerate() {
        if !job.sessions_current() {
            return job.ctx.failed(CoreError::Superseded);
        }
        if job.cancel.is_cancelled() {
            return cancelled;
        }
        match count_item(job, index, item).await {
            Ok(Some(counts)) => emit_counts(job, index, Some(counts)),
            Ok(None) if job.is_lost() => return job.ctx.failed(CoreError::Superseded),
            Ok(None) => return cancelled,
            Err(CoreError::CompareCapExceeded) => emit_counts(job, index, None),
            Err(error) => return job.ctx.failed(error),
        }
    }
    CopyEvent::PreviewDone { run }
}

fn emit_counts(job: &Job, index: usize, counts: Option<PreviewCounts>) {
    job.ctx.emit(CopyEvent::PreviewItemDone {
        run: job.ctx.run,
        index,
        counts,
    });
}

/// `Ok(None)` when the count was cancelled.
async fn count_item(job: &Job, index: usize, item: &CopyItem) -> Result<Option<PreviewCounts>> {
    let mut on_progress = |scanned| {
        job.ctx.emit(CopyEvent::PreviewProgress {
            run: job.ctx.run,
            index,
            scanned,
        });
        job.sessions_current();
    };
    let (outcome, counts) =
        preview_collection(&job.copy_job(item), &job.cancel, &mut on_progress).await?;
    Ok((outcome == EngineOutcome::Completed).then_some(counts))
}
