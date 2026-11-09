//! Managing the saved undo snapshots: listing, deleting and expiry. None of it needs a session.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use super::{Event, Repaint, Shared};
use crate::bulk::BulkRunId;
use crate::snapshot::{self, RETENTION_SECS};

/// Removes snapshots past their retention in the background. A failure is logged without a path.
pub(super) fn spawn_expiry(dir: Option<PathBuf>) {
    let Some(dir) = dir else {
        return;
    };
    tokio::task::spawn_blocking(move || {
        snapshot::purge_older_than(&dir, crate::clock::now_secs(), RETENTION_SECS);
    });
}

/// Deletes `delete` first when given, then answers with the listing. With undo off (no
/// directory) the listing is empty.
pub(super) fn spawn_listing(
    shared: &Arc<Shared>,
    events: UnboundedSender<Event>,
    repaint: Repaint,
    request: u64,
    delete: Option<BulkRunId>,
) {
    let dir = shared.config.snapshot_dir.clone();
    tokio::spawn(async move {
        let snapshots = tokio::task::spawn_blocking(move || {
            let dir = dir?;
            if let Some(run) = delete
                && snapshot::delete(&dir, run).is_err()
            {
                log::warn!("a snapshot could not be deleted");
            }
            Some(snapshot::list(&dir))
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        if events.send(Event::Snapshots { request, snapshots }).is_ok() {
            repaint();
        }
    });
}
