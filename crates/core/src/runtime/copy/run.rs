//! The copy loop of a `Start`: one collection after another through the engine.

use super::{CopyEvent, CopyItem, Job};
use crate::copy::{CopyOutcome, CopyReport, copy_with_structure};
use crate::error::CoreError;

/// Runs every item; returns the terminal event.
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
        let report = copy_item(job, index, item).await;
        let outcome = report.outcome.clone();
        job.ctx.emit(CopyEvent::ItemDone { run, index, report });
        match outcome {
            CopyOutcome::Completed => {}
            // The cancel may be ours, after a session was lost: that is a failure, not a cancel.
            _ if job.is_lost() => return job.ctx.failed(CoreError::Superseded),
            CopyOutcome::Cancelled => return cancelled,
            CopyOutcome::Failed(error) => return job.ctx.failed(error),
        }
    }
    CopyEvent::Done {
        run,
        cancelled: false,
    }
}

async fn copy_item(job: &Job, index: usize, item: &CopyItem) -> CopyReport {
    let run = job.ctx.run;
    let estimated_total = estimate(job, item).await;
    let mut on_progress = |progress| {
        job.ctx.emit(CopyEvent::Progress {
            run,
            index,
            progress,
            estimated_total,
        });
        // After the event, so the counts of the batch just written are delivered either way.
        job.sessions_current();
    };
    copy_with_structure(
        job.copy_job(item),
        job.with_indexes_and_options,
        &job.cancel,
        &mut on_progress,
    )
    .await
}

/// The progress denominator: the estimated count of the whole collection. A filtered copy has
/// no usable estimate, and a failed estimate is only a missing denominator.
async fn estimate(job: &Job, item: &CopyItem) -> Option<u64> {
    if !item.filter.is_empty() {
        return None;
    }
    job.source.store.estimated_count(&item.source).await.ok()
}
