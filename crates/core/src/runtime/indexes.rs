//! The runtime side of index usage history: opening the database, answering `IndexHistory`,
//! purging a connection and running a connection's sampler.

use std::path::Path;
use std::sync::Arc;

use uuid::Uuid;

use super::sampler;
use super::session::Session;
use super::{Event, Repaint, SessionId, Shared};
use crate::clock;
use crate::error::{CoreError, Result};
use crate::history::{Bucket, ChartRange, HistoryDb, Recovery, Sample};
use crate::store::Namespace;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// One plotted series of the Indexes tab: an index definition on one node.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexSeries {
    pub index_name: String,
    /// Tells two definitions of one index name apart (an index dropped and recreated).
    pub definition_hash: String,
    pub host: String,
    pub buckets: Vec<Bucket>,
    /// The newest recorded reading, for the "uses since" line.
    pub latest: Option<Sample>,
}

/// Opens the history database. Called once, synchronously, when the runtime is spawned: a
/// SQLite open is a few milliseconds and the spawn functions are not async. A failure, or a
/// corrupt file that had to be set aside, is logged; on failure history is off for this run
/// rather than the app failing to start.
pub(super) fn open_history(path: Option<&Path>) -> Option<Arc<HistoryDb>> {
    let path = path?;
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::warn!("usage history disabled: cannot create {parent:?}: {error}");
            return None;
        }
    }
    match HistoryDb::open(path) {
        Ok((db, recovery)) => {
            if let Some(Recovery::SetAside(aside)) = recovery {
                log::warn!(
                    "usage history was unreadable and started afresh; the old file is at {}",
                    aside.display()
                );
            }
            Some(Arc::new(db))
        }
        Err(error) => {
            log::warn!("usage history disabled: {error}");
            None
        }
    }
}

/// Every series stored for the collection, newest data of `range`. No history means no series.
pub(super) async fn series_for(
    history: Option<Arc<HistoryDb>>,
    session: SessionId,
    ns: &Namespace,
    range: ChartRange,
) -> Result<Vec<IndexSeries>> {
    let Some(history) = history else {
        return Ok(Vec::new());
    };
    let ns = ns.clone();
    tokio::task::spawn_blocking(move || {
        let now = clock::now_secs();
        history
            .series_for(session, &ns.db, &ns.collection)?
            .into_iter()
            .map(|key| {
                Ok(IndexSeries {
                    buckets: history.usage(&key, range, now)?,
                    latest: history.latest(&key)?,
                    index_name: key.index_name,
                    definition_hash: key.definition_hash,
                    host: key.host,
                })
            })
            .collect()
    })
    .await
    .map_err(|_| CoreError::Database("internal error".into()))?
}

/// Deletes a connection's history in the background; there is no answer, a failure is logged.
pub(super) fn spawn_purge(shared: &Shared, connection: Uuid) {
    let Some(history) = shared.history.clone() else {
        return;
    };
    tokio::spawn(async move {
        let purged =
            tokio::task::spawn_blocking(move || history.purge_connection(connection)).await;
        match purged {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("purging usage history of {connection} failed: {error}"),
            Err(error) => log::warn!("purging usage history of {connection} panicked: {error}"),
        }
    });
}

/// Starts the sampler of a session that just connected, if history is available and the connect
/// is still current, and forwards its statuses as `Event::Sampler`. The sampler reads through
/// the session's own (read-only wrapped) store; sampling only reads.
pub(super) fn start_sampler(
    shared: &Shared,
    session: &Arc<Session>,
    id: SessionId,
    generation: u64,
    events: UnboundedSender<Event>,
    repaint: Repaint,
) {
    let (Some(history), Some(store)) = (shared.history.clone(), session.current_store()) else {
        return;
    };
    let (status_tx, mut statuses) = unbounded_channel();
    let handle = sampler::spawn(
        store,
        history,
        id,
        shared.sampler_config.subscribe(),
        status_tx,
    );
    if !session.attach_sampler(generation, handle) {
        return;
    }
    let session = session.clone();
    // Ends when the sampler is aborted (its sender drops) or the event channel closes.
    tokio::spawn(async move {
        while let Some(status) = statuses.recv().await {
            // A status queued before an abort belongs to a connection the person already left.
            if !session.is_current(generation) {
                break;
            }
            if events
                .send(Event::Sampler {
                    session: id,
                    status,
                })
                .is_err()
            {
                break;
            }
            repaint();
        }
    });
}
