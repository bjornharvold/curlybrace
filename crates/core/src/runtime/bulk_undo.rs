//! Undo for bulk runs: the snapshot taken before a write, its after-state captured once the write
//! finished, and the preview and restore that use them.
//!
//! A snapshot holds the person's documents in clear text: nothing here logs a path or a value,
//! and a snapshot is only ever restored on the connection it was taken from.

use std::path::PathBuf;
use std::sync::Arc;

use bson::{Bson, Document};

use super::Shared;
use super::handlers::bounded;
use crate::bulk::{BULK_PREVIEW_DOCS, BulkOutcome, BulkRunId, UndoOffer, UndoPolicy, UndoStatus};
use crate::bulk::{UndoPreview, UndoPreviewDoc};
use crate::error::{CoreError, Result};
use crate::snapshot::{
    SnapshotHeader, SnapshotKind, SnapshotReader, SnapshotWriter, UndoReport, after_path_for,
    capture_after, run_undo, snapshot_path,
};
use crate::store::{DocumentStore, Namespace};

/// A snapshot written before a write, not yet known to be needed.
pub(super) struct Taken {
    run: BulkRunId,
    path: PathBuf,
    documents: u64,
    /// An update also needs its after-state saved; a delete does not.
    is_update: bool,
}

fn dir_of(shared: &Shared) -> Result<PathBuf> {
    shared
        .config
        .snapshot_dir
        .clone()
        .ok_or(CoreError::SnapshotWriteFailed)
}

/// How many bytes the originals of a run may take. An update later adds its after-state to the
/// same budget, so it gets half; a delete needs no after-state and gets all of it.
fn originals_budget(max_bytes: u64, is_update: bool) -> u64 {
    if is_update { max_bytes / 2 } else { max_bytes }
}

/// Saves the originals of the documents `kind` will touch. `Ok(None)` when the person chose to
/// run without undo. An error means the write must not run: nothing is left on disk.
pub(super) async fn take(
    shared: &Shared,
    store: &Arc<dyn DocumentStore>,
    connection: uuid::Uuid,
    ns: &Namespace,
    kind: SnapshotKind,
    run: BulkRunId,
    policy: UndoPolicy,
) -> Result<Option<Taken>> {
    if policy == UndoPolicy::None {
        return Ok(None);
    }
    let (filter, is_update) = match &kind {
        SnapshotKind::Update { filter, .. } => (filter.clone(), true),
        SnapshotKind::Delete { filter } => (filter.clone(), false),
    };
    let header = SnapshotHeader {
        run,
        connection,
        ns: ns.clone(),
        kind,
        created_unix: crate::clock::now_secs(),
    };
    let budget = originals_budget(shared.config.snapshot_max_bytes, is_update);
    let mut writer = SnapshotWriter::create(&dir_of(shared)?, header, budget)?;
    let stream = store.snapshot_matching(ns, &filter, run, &mut writer);
    // Dropping the writer on any error below removes the partial file.
    let documents = match tokio::time::timeout(shared.config.bulk_write_timeout, stream).await {
        Ok(Ok(documents)) => documents,
        Ok(Err(e)) => return Err(snapshot_failure(e)),
        Err(_) => return Err(CoreError::SnapshotWriteFailed),
    };
    let path = tokio::task::spawn_blocking(move || writer.finish())
        .await
        .map_err(|_| CoreError::SnapshotWriteFailed)??;
    Ok(Some(Taken {
        run,
        path,
        documents,
        is_update,
    }))
}

/// A snapshot error stays itself; anything the store said while streaming is a write failure of
/// the snapshot (its text could quote a value).
fn snapshot_failure(error: CoreError) -> CoreError {
    match error {
        CoreError::SnapshotTooLarge | CoreError::SnapshotWriteFailed | CoreError::ReadOnly => error,
        _ => CoreError::SnapshotWriteFailed,
    }
}

/// How many documents the write reports it changed or removed, when it says.
fn touched(outcome: &BulkOutcome, is_update: bool) -> Option<u64> {
    match outcome {
        BulkOutcome::Applied {
            matched, deleted, ..
        }
        | BulkOutcome::Unconfirmed {
            matched, deleted, ..
        } => Some(if is_update { *matched } else { *deleted }),
        BulkOutcome::Unknown { .. } | BulkOutcome::Rejected { .. } => None,
    }
}

/// Decides what the write left behind. A write that never ran leaves nothing; a run that touched
/// nothing leaves nothing; otherwise the after-state is captured and undo is offered, together
/// with how many documents the write changed that the snapshot does not hold (matches that
/// appeared between the snapshot and the write). If the after-state cannot be saved the loss is
/// reported, not hidden.
pub(super) async fn seal(
    shared: &Shared,
    store: &Arc<dyn DocumentStore>,
    taken: Option<Taken>,
    outcome: &Result<BulkOutcome>,
) -> UndoStatus {
    let Some(taken) = taken else {
        return UndoStatus::NotRequested;
    };
    let upserted: Vec<Bson> = match outcome {
        Ok(BulkOutcome::Applied { upserted, .. } | BulkOutcome::Unconfirmed { upserted, .. }) => {
            upserted.clone()
        }
        _ => Vec::new(),
    };
    if outcome.is_err() || (taken.documents == 0 && upserted.is_empty()) {
        discard(shared, &taken);
        return UndoStatus::NothingToUndo;
    }
    if taken.is_update && capture(shared, store, &taken, &upserted).await.is_err() {
        // Without the after-state an update cannot be undone safely.
        log::warn!("undo: the state after the write could not be saved");
        discard(shared, &taken);
        return UndoStatus::Lost;
    }
    let uncovered = outcome
        .as_ref()
        .ok()
        .and_then(|o| touched(o, taken.is_update))
        .map_or(0, |n| n.saturating_sub(taken.documents));
    UndoStatus::Offered(UndoOffer {
        documents: taken.documents,
        uncovered,
    })
}

async fn capture(
    shared: &Shared,
    store: &Arc<dyn DocumentStore>,
    taken: &Taken,
    upserted: &[Bson],
) -> Result<()> {
    let used = std::fs::metadata(&taken.path).map_or(0, |m| m.len());
    let room = shared.config.snapshot_max_bytes.saturating_sub(used);
    let work = capture_after(store.as_ref(), &taken.path, upserted, room);
    match tokio::time::timeout(shared.config.bulk_write_timeout, work).await {
        Ok(result) => result,
        Err(_) => Err(CoreError::SnapshotWriteFailed),
    }
}

/// Removes a snapshot that is not needed (or could not be completed).
pub(super) fn discard_taken(shared: &Shared, taken: &Taken) {
    discard(shared, taken);
}

fn discard(shared: &Shared, taken: &Taken) {
    let Some(dir) = shared.config.snapshot_dir.as_deref() else {
        return;
    };
    if crate::snapshot::delete(dir, taken.run).is_err() {
        log::warn!("undo: a snapshot that is no longer needed could not be removed");
    }
}

/// Opens the run's snapshot, refusing one that belongs to another connection.
fn open_for(
    shared: &Shared,
    connection: uuid::Uuid,
    run: BulkRunId,
) -> Result<(PathBuf, SnapshotReader)> {
    let path = snapshot_path(&dir_of(shared)?, run);
    let reader = SnapshotReader::open(&path)?;
    if reader.header().connection != connection {
        return Err(CoreError::SnapshotOtherConnection);
    }
    Ok((path, reader))
}

/// The namespace, size and first documents of a snapshot (a full scan, so on the blocking pool).
fn scan(
    shared: &Shared,
    connection: uuid::Uuid,
    run: BulkRunId,
) -> Result<impl FnOnce() -> Result<(Namespace, u64, Vec<Document>)> + Send + use<>> {
    let (_, reader) = open_for(shared, connection, run)?;
    Ok(move || {
        let ns = reader.header().ns.clone();
        let mut documents = 0_u64;
        let mut first: Vec<Document> = Vec::with_capacity(BULK_PREVIEW_DOCS);
        for original in reader {
            let original = original?;
            documents += 1;
            if first.len() < BULK_PREVIEW_DOCS {
                first.push(original);
            }
        }
        Ok((ns, documents, first))
    })
}

/// The size of the run's snapshot and its first documents beside what they are now.
pub(super) async fn preview(
    shared: &Shared,
    store: &Arc<dyn DocumentStore>,
    connection: uuid::Uuid,
    run: BulkRunId,
) -> Result<UndoPreview> {
    let work = scan(shared, connection, run)?;
    let (ns, documents, first) = tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CoreError::SnapshotUnreadable)??;
    let ids: Vec<Bson> = first.iter().filter_map(|d| d.get("_id").cloned()).collect();
    let current = bounded(shared.config.command_timeout, store.find_by_ids(&ns, &ids)).await?;
    let docs = first
        .into_iter()
        .map(|original| UndoPreviewDoc {
            current: current
                .iter()
                .find(|c| c.get("_id") == original.get("_id"))
                .cloned(),
            original,
        })
        .collect();
    Ok(UndoPreview { documents, docs })
}

/// Restores the run's documents. The snapshot stays, so a partial or failed undo can be retried.
pub(super) async fn restore(
    shared: &Shared,
    store: &Arc<dyn DocumentStore>,
    connection: uuid::Uuid,
    run: BulkRunId,
) -> Result<UndoReport> {
    let (path, _) = open_for(shared, connection, run)?;
    let after = after_path_for(&path);
    let after = after.exists().then_some(after);
    let work = run_undo(store.as_ref(), &path, after.as_deref());
    match tokio::time::timeout(shared.config.bulk_write_timeout, work).await {
        Ok(Err(CoreError::ConnectionInterrupted(_))) | Err(_) => {
            Err(CoreError::WriteOutcomeUnknown)
        }
        Ok(other) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_reserves_half_the_cap_for_its_after_state() {
        assert_eq!(originals_budget(1000, true), 500);
        assert_eq!(originals_budget(1000, false), 1000);
    }
}
