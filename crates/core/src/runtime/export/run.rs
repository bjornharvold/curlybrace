//! The export loop: create the file, stream batches into it, then keep or remove it.

use std::path::Path;

use super::{Job, after_removal};
use crate::error::{CoreError, Result};
use crate::export::protocol::{DocExportOutcome, ExportEvent};
use crate::export::read::{DocReader, EXPORT_BATCH};
use crate::export::render::DocWriter;
use crate::runtime::export_file::{ExportSink, FileGuard, Out, buffered, into_file};

/// How the streaming ended without an error.
enum Streamed {
    /// Every document was written; the buffer still has to be flushed and synced.
    Finished { out: Out, written: u64 },
    /// Cancelled, or the session left its epoch (see `Job::is_lost`).
    Stopped,
}

/// Runs the export; returns the terminal event, which the caller sends once the slot is free.
pub(super) async fn execute(job: Job) -> ExportEvent {
    let path = job.request.path.clone();
    let (file, guard) = match create_file(&path).await {
        Ok(created) => created,
        Err(error) => return job.ctx.failed(error),
    };
    let Some(total) = job.cancel.run_until_cancelled(estimate(&job)).await else {
        return stopped(&job, guard);
    };
    let out = buffered(ExportSink::new(file, &path));
    match stream(&job, out, total).await {
        Ok(Streamed::Finished { out, written }) => finish(&job, guard, out, written).await,
        Ok(Streamed::Stopped) => stopped(&job, guard),
        Err(error) => job.ctx.failed(after_removal(guard, error)),
    }
}

async fn create_file(path: &Path) -> Result<(std::fs::File, FileGuard)> {
    let owned = path.to_path_buf();
    tokio::task::spawn_blocking(move || FileGuard::create(&owned))
        .await
        .map_err(|_| CoreError::Database("internal error".into()))?
}

/// The progress denominator: the estimated count for a whole collection, the exact count for a
/// filter. Either may be unavailable, which is only a missing denominator.
async fn estimate(job: &Job) -> Option<u64> {
    let request = &job.request;
    if request.filter.is_empty() {
        return job.store.estimated_count(&request.ns).await.ok();
    }
    job.store
        .count_matching(&request.ns, &request.filter, job.limit)
        .await
        .ok()
        .flatten()
}

fn write_failed(_: std::io::Error) -> CoreError {
    CoreError::ExportWriteFailed
}

/// Reads batch after batch and renders each document into `out`. Stops before a batch when
/// cancelled or when the session left its epoch.
async fn stream(job: &Job, out: Out, total: Option<u64>) -> Result<Streamed> {
    let request = &job.request;
    let mut writer = DocWriter::new(out, request.format, request.mode);
    writer.begin().map_err(write_failed)?;
    let mut reader = DocReader::new(
        job.store.clone(),
        request.ns.clone(),
        request.filter.clone(),
        EXPORT_BATCH,
    );
    let mut written = 0_u64;
    loop {
        if job.stopped() {
            return Ok(Streamed::Stopped);
        }
        let Some(read) = job.cancel.run_until_cancelled(reader.next_batch()).await else {
            return Ok(Streamed::Stopped);
        };
        let Some(batch) = read? else { break };
        for doc in &batch {
            writer.doc(doc).map_err(write_failed)?;
        }
        written += batch.len() as u64;
        job.ctx.emit(ExportEvent::Progress {
            run: job.ctx.run,
            done: total.map_or(written, |t| written.min(t)),
            total,
        });
    }
    if job.stopped() {
        return Ok(Streamed::Stopped);
    }
    let out = writer.finish().map_err(write_failed)?;
    Ok(Streamed::Finished { out, written })
}

/// Flushes and syncs the file, then keeps it.
async fn finish(job: &Job, guard: FileGuard, out: Out, written: u64) -> ExportEvent {
    let synced = tokio::task::spawn_blocking(move || {
        let file = into_file(out)?;
        file.sync_all()?;
        file.metadata().map(|m| m.len())
    })
    .await;
    match synced {
        Ok(Ok(bytes)) => {
            guard.commit();
            ExportEvent::Done {
                run: job.ctx.run,
                outcome: DocExportOutcome {
                    written,
                    bytes,
                    cancelled: false,
                    file_left: false,
                },
            }
        }
        Ok(Err(_)) => job
            .ctx
            .failed(after_removal(guard, CoreError::ExportWriteFailed)),
        Err(_) => job.ctx.failed(after_removal(
            guard,
            CoreError::Database("internal error".into()),
        )),
    }
}

/// The run was cancelled or lost its session: the file goes. A lost session is a failure, a
/// cancel is not.
fn stopped(job: &Job, guard: FileGuard) -> ExportEvent {
    if job.is_lost() {
        return job.ctx.failed(after_removal(guard, CoreError::Superseded));
    }
    let removed = guard.remove();
    ExportEvent::Done {
        run: job.ctx.run,
        outcome: DocExportOutcome {
            cancelled: true,
            file_left: !removed,
            ..DocExportOutcome::default()
        },
    }
}
