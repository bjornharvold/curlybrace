//! The runtime side of index usage history: opening the database, answering `IndexHistory`,
//! purging a connection and running a connection's sampler.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use uuid::Uuid;

use super::sampler;
use super::session::Session;
use super::{Event, Repaint, SessionId, Shared};
use crate::clock;
use crate::error::{CoreError, Result};
use crate::history::{Bucket, ChartRange, HistoryDb, Recovery, Sample};
use crate::store::{CollectionIndex, Namespace};
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

/// The series stored for the collection's `listed` indexes (every node of each definition),
/// newest data of `range`. Series of dropped or recreated definitions are not read: the tab
/// draws one series per listed index, and every extra one is a full scan under the database
/// lock. No history means no series.
pub(super) async fn series_for(
    history: Option<Arc<HistoryDb>>,
    session: SessionId,
    ns: &Namespace,
    range: ChartRange,
    listed: &[CollectionIndex],
) -> Result<Vec<IndexSeries>> {
    let Some(history) = history else {
        return Ok(Vec::new());
    };
    let ns = ns.clone();
    let listed: HashSet<(String, String)> = listed
        .iter()
        .map(|index| (index.spec.name.clone(), index.spec.definition_hash()))
        .collect();
    tokio::task::spawn_blocking(move || {
        let now = clock::now_secs();
        history
            .series_for(session, &ns.db, &ns.collection)?
            .into_iter()
            .filter(|key| listed.contains(&(key.index_name.clone(), key.definition_hash.clone())))
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
    if shared.history.is_none() && shared.config.history_path.is_some() {
        // History was wanted but could not be opened (logged at spawn): tell the person instead
        // of leaving the tab to promise sampling that never happens.
        let status = sampler::SamplerStatus::Paused(sampler::SamplerPause::HistoryUnavailable);
        if let Forwarded::Sent = forward(&Arc::downgrade(session), generation, &events, id, status)
        {
            repaint();
        }
        return;
    }
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
    let abort = handle.abort_handle();
    if !session.attach_sampler(generation, handle) {
        return;
    }
    // Weak: a strong reference would keep the session (and so the sampler and its store) alive
    // for as long as the sampler runs, a cycle that outlives the runtime.
    let session = Arc::downgrade(session);
    // Ends when the sampler is aborted (its sender drops), the session is gone or the event
    // channel closes; in the last two cases it stops the sampler itself.
    tokio::spawn(async move {
        loop {
            let status = tokio::select! {
                status = statuses.recv() => status,
                () = events.closed() => None,
            };
            let Some(status) = status else {
                abort.abort();
                return;
            };
            match forward(&session, generation, &events, id, status) {
                Forwarded::Sent => repaint(),
                Forwarded::Stale => return,
                Forwarded::Closed => {
                    abort.abort();
                    return;
                }
            }
        }
    });
}

enum Forwarded {
    Sent,
    /// The session moved on (or is gone): the sampler is already aborted, the status is dropped.
    Stale,
    /// Nobody receives events any more.
    Closed,
}

/// Sends one status unless the session left the epoch the sampler was started in. The currentness
/// check and the send happen under the READ side of the session's `epoch_lock`, like
/// `handlers::send_answer`, so `Session::begin` cannot slip in between: a status is never
/// delivered after the `Disconnected` or the newer `Connected` of its session. Synchronous, so the
/// std lock is never held across an `.await`.
fn forward(
    session: &std::sync::Weak<Session>,
    generation: u64,
    events: &UnboundedSender<Event>,
    id: SessionId,
    status: sampler::SamplerStatus,
) -> Forwarded {
    let Some(session) = session.upgrade() else {
        return Forwarded::Stale;
    };
    let _reading = session.epoch_lock.read().unwrap_or_else(|e| e.into_inner());
    if !session.is_current(generation) {
        return Forwarded::Stale;
    }
    match events.send(Event::Sampler {
        session: id,
        status,
    }) {
        Ok(()) => Forwarded::Sent,
        Err(_) => Forwarded::Closed,
    }
}
