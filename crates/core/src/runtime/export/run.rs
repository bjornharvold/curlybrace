//! The export loop: read the first batch, create the file, stream batches into it, then keep or
//! remove it.

use std::path::Path;

use bson::Document;

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
///
/// Everything that can fail early (the count, the first batch) happens BEFORE the file is created
/// or truncated, so an early failure or cancel leaves a file already at the path untouched.
pub(super) async fn execute(job: Job) -> ExportEvent {
    let path = job.request.path.clone();
    let Some(total) = job.cancel.run_until_cancelled(estimate(&job)).await else {
        return stopped(&job, None);
    };
    let mut reader = DocReader::new(
        job.store.clone(),
        job.request.ns.clone(),
        job.request.filter.clone(),
        EXPORT_BATCH,
    );
    let first = match job.cancel.run_until_cancelled(reader.next_batch()).await {
        None => return stopped(&job, None),
        Some(Err(error)) => return job.ctx.failed(error),
        Some(Ok(first)) => first,
    };
    if job.stopped() {
        return stopped(&job, None);
    }
    let (file, guard) = match create_file(&path).await {
        Ok(created) => created,
        Err(error) => return job.ctx.failed(error),
    };
    let out = buffered(ExportSink::new(file, &path));
    match stream(&job, out, (reader, first), total).await {
        Ok(Streamed::Finished { out, written }) => finish(&job, guard, out, written).await,
        Ok(Streamed::Stopped) => stopped(&job, Some(guard)),
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
        let count = tokio::time::timeout(job.limit, job.store.estimated_count(&request.ns));
        return count.await.ok().and_then(|count| count.ok());
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

/// Renders the first batch, then reads batch after batch into `out`. Stops before a read when
/// cancelled or when the session left its epoch. `total` is only a hint: once more documents
/// than it have been written it is dropped from the progress.
async fn stream(
    job: &Job,
    out: Out,
    (mut reader, first): (DocReader, Option<Vec<Document>>),
    total: Option<u64>,
) -> Result<Streamed> {
    let request = &job.request;
    let mut writer = DocWriter::new(out, request.format, request.mode);
    writer.begin().map_err(write_failed)?;
    let mut written = 0_u64;
    let mut next = first;
    while let Some(batch) = next {
        written += batch.len() as u64;
        for doc in batch {
            writer.doc(doc).map_err(write_failed)?;
        }
        job.ctx.emit(ExportEvent::Progress {
            run: job.ctx.run,
            done: written,
            total: total.filter(|t| written <= *t),
        });
        if job.stopped() {
            return Ok(Streamed::Stopped);
        }
        next = match job.cancel.run_until_cancelled(reader.next_batch()).await {
            Some(read) => read?,
            None => return Ok(Streamed::Stopped),
        };
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

/// The run was cancelled or lost its session: the file, if one was created, goes. A lost session
/// is a failure, a cancel is not.
fn stopped(job: &Job, guard: Option<FileGuard>) -> ExportEvent {
    if job.is_lost() {
        let error = match guard {
            Some(guard) => after_removal(guard, CoreError::Superseded),
            None => CoreError::Superseded,
        };
        return job.ctx.failed(error);
    }
    let removed = guard.is_none_or(FileGuard::remove);
    ExportEvent::Done {
        run: job.ctx.run,
        outcome: DocExportOutcome {
            cancelled: true,
            file_left: !removed,
            ..DocExportOutcome::default()
        },
    }
}
