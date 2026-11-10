//! The bulk commands: counting, sampling, previewing, applying and cancelling a bulk update or
//! delete. Each is a read or a write of the session's store; a write that outruns its bound or
//! loses its connection is an unknown outcome, not a failure, and is never settled by a ping.

use std::sync::Arc;
use std::time::Duration;

use super::bulk_undo;
use super::handlers::{bounded, gate};
use super::session::Session;
use super::{BULK_COUNT_MAX_TIME, Command, Event, SessionId, Shared};
use crate::bulk::{BULK_PREVIEW_DOCS, BulkOutcome, UndoPolicy, UndoStatus};
use crate::bulk::{BulkDeleteRequest, BulkRunId, BulkUpdateRequest};
use crate::error::{CoreError, Result};
use crate::snapshot::SnapshotKind;
use crate::store::Namespace;
use crate::store::{DocumentStore, Query};
use bson::Document;

/// A bulk command with only the fields its handler needs, split by what its answer means.
pub(super) enum Bulk {
    Read(ReadBulk),
    Write(WriteBulk),
}

pub(super) enum ReadBulk {
    Count {
        ns: Namespace,
        filter: Document,
        request: u64,
    },
    Sample {
        ns: Namespace,
        filter: Document,
        request: u64,
    },
    Preview {
        ns: Namespace,
        request: BulkUpdateRequest,
        request_id: u64,
    },
    Cancel {
        run: BulkRunId,
    },
    UndoPreview {
        run: BulkRunId,
        request_id: u64,
    },
}

pub(super) enum WriteBulk {
    Update {
        ns: Namespace,
        request: BulkUpdateRequest,
        run: BulkRunId,
        undo: UndoPolicy,
    },
    Delete {
        ns: Namespace,
        request: BulkDeleteRequest,
        run: BulkRunId,
        undo: UndoPolicy,
    },
    Undo {
        run: BulkRunId,
    },
}

impl Bulk {
    #[allow(clippy::result_large_err)] // the error is the unconsumed command, moved once
    /// The bulk form of `command`, or the command back when it is not a bulk one.
    pub(super) fn from_command(command: Command) -> std::result::Result<Self, Command> {
        Ok(match command {
            Command::BulkCount {
                ns,
                filter,
                request,
                ..
            } => Self::Read(ReadBulk::Count {
                ns,
                filter,
                request,
            }),
            Command::BulkSample {
                ns,
                filter,
                request,
                ..
            } => Self::Read(ReadBulk::Sample {
                ns,
                filter,
                request,
            }),
            Command::BulkPreview {
                ns,
                request,
                request_id,
                ..
            } => Self::Read(ReadBulk::Preview {
                ns,
                request,
                request_id,
            }),
            Command::BulkCancel { run, .. } => Self::Read(ReadBulk::Cancel { run }),
            Command::BulkUpdate {
                ns,
                request,
                run,
                undo,
                ..
            } => Self::Write(WriteBulk::Update {
                ns,
                request,
                run,
                undo,
            }),
            Command::BulkDelete {
                ns,
                request,
                run,
                undo,
                ..
            } => Self::Write(WriteBulk::Delete {
                ns,
                request,
                run,
                undo,
            }),
            Command::BulkUndoPreview {
                run, request_id, ..
            } => Self::Read(ReadBulk::UndoPreview { run, request_id }),
            Command::BulkUndo { run, .. } => Self::Write(WriteBulk::Undo { run }),
            other => return Err(other),
        })
    }
}

/// Runs a bulk command. An `Err` is a failure of a read (or a superseded session); every failure
/// of a write is an outcome of its `BulkFinished`.
pub(super) async fn run(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    id: SessionId,
    store: Arc<dyn DocumentStore>,
    generation: u64,
    bulk: Bulk,
) -> Result<Event> {
    // A command for a replaced connection must not run against the new one's store.
    if !session.is_current(generation) {
        return Err(CoreError::Superseded);
    }
    match bulk {
        // A write that was started always reports its outcome, even when the session was
        // replaced while it ran: the answer is the only report of changes the server may have
        // applied, and `finalize` lets `BulkFinished` through for the same reason.
        Bulk::Write(write) => Ok(run_write(shared, id, &store, write).await),
        Bulk::Read(read) => {
            let event = run_read(shared, id, &store, read)
                .await
                .map_err(|e| gate(session, generation, e))?;
            if session.is_current(generation) {
                Ok(event)
            } else {
                Err(CoreError::Superseded)
            }
        }
    }
}

async fn run_read(
    shared: &Shared,
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    read: ReadBulk,
) -> Result<Event> {
    let limit = shared.config.command_timeout;
    match read {
        ReadBulk::Count {
            ns,
            filter,
            request,
        } => count(limit, session, store, ns, filter, request).await,
        ReadBulk::Sample {
            ns,
            filter,
            request,
        } => sample(limit, session, store, ns, filter, request).await,
        ReadBulk::Preview {
            ns,
            request,
            request_id,
        } => Ok(Event::BulkPreview {
            session,
            outcome: bounded(limit, store.preview_update(&ns, &request)).await,
            ns,
            request_id,
        }),
        ReadBulk::UndoPreview { run, request_id } => Ok(Event::BulkUndoPreview {
            session,
            run,
            request_id,
            outcome: bulk_undo::preview(shared, store, session, run).await,
        }),
        ReadBulk::Cancel { run } => {
            // Before the kill, so a run still waiting or snapshotting sees it and does not write.
            mark_cancelled(shared, run);
            Ok(Event::BulkCancelResult {
                session,
                run,
                outcome: bounded(limit, store.cancel_bulk(run)).await,
            })
        }
    }
}

async fn count(
    limit: Duration,
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    ns: Namespace,
    filter: Document,
    request: u64,
) -> Result<Event> {
    let max = BULK_COUNT_MAX_TIME;
    let count = bounded(limit, store.count_matching(&ns, &filter, max)).await?;
    Ok(Event::BulkCount {
        session,
        ns,
        request,
        count,
    })
}

/// The first documents the filter matches. Straight to the store: the page's own find path
/// records query history.
async fn sample(
    limit: Duration,
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    ns: Namespace,
    filter: Document,
    request: u64,
) -> Result<Event> {
    let query = Query {
        filter,
        limit: BULK_PREVIEW_DOCS as u64,
        ..Query::default()
    };
    let page = bounded(limit, store.find(&ns, &query)).await?;
    Ok(Event::BulkSample {
        session,
        ns,
        request,
        docs: page.docs,
    })
}

async fn run_write(
    shared: &Shared,
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    write: WriteBulk,
) -> Event {
    match write {
        WriteBulk::Update {
            ns,
            request,
            run,
            undo,
        } => {
            let kind = SnapshotKind::Update {
                filter: request.filter.clone(),
                update: request.update.clone(),
                upsert: request.upsert,
            };
            let apply = || store.update_many(&ns, &request, run);
            finish(shared, session, store, ns.clone(), kind, run, undo, apply).await
        }
        WriteBulk::Delete {
            ns,
            request,
            run,
            undo,
        } => {
            let kind = SnapshotKind::Delete {
                filter: request.filter.clone(),
            };
            let apply = || store.delete_many(&ns, &request, run);
            finish(shared, session, store, ns.clone(), kind, run, undo, apply).await
        }
        WriteBulk::Undo { run } => Event::BulkUndoFinished {
            session,
            run,
            outcome: bulk_undo::restore(shared, store, session, run).await,
        },
    }
}

/// Snapshots (when asked), writes, then seals the snapshot. A snapshot that cannot be taken ends
/// the run before anything is sent to the server, and so does a cancel that arrived while the
/// run waited or saved its snapshot.
#[allow(clippy::too_many_arguments)]
async fn finish<F, Fut>(
    shared: &Shared,
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    ns: Namespace,
    kind: SnapshotKind,
    run: BulkRunId,
    policy: UndoPolicy,
    apply: F,
) -> Event
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<BulkOutcome>>,
{
    let finished = |outcome, undo| Event::BulkFinished {
        session,
        ns: ns.clone(),
        run,
        outcome,
        undo,
    };
    let taken = bulk_undo::take(shared, store, session, &ns, kind, run, policy).await;
    if cancel_requested(shared, run) {
        if let Ok(Some(taken)) = &taken {
            bulk_undo::discard_taken(shared, taken);
        }
        clear_cancel(shared, run);
        return finished(Err(CoreError::BulkCancelled), UndoStatus::NotRequested);
    }
    let taken = match taken {
        Ok(taken) => taken,
        Err(error) => return finished(Err(error), UndoStatus::NotRequested),
    };
    let outcome = write_outcome(shared, store, run, apply()).await;
    let undo = bulk_undo::seal(shared, store, taken, &outcome).await;
    clear_cancel(shared, run);
    finished(outcome, undo)
}

fn mark_cancelled(shared: &Shared, run: BulkRunId) {
    if let Ok(mut set) = shared.cancelled.lock() {
        set.insert(run.0);
    }
}

fn cancel_requested(shared: &Shared, run: BulkRunId) -> bool {
    shared
        .cancelled
        .lock()
        .is_ok_and(|set| set.contains(&run.0))
}

fn clear_cancel(shared: &Shared, run: BulkRunId) {
    if let Ok(mut set) = shared.cancelled.lock() {
        set.remove(&run.0);
    }
}

/// A write's outcome: outrunning the bound or losing the connection leaves the result unknown.
/// When the bound expires the future is dropped but the server keeps writing, so the run is
/// cancelled there first (best effort: a failure is logged without values and the unknown
/// outcome stands).
async fn write_outcome(
    shared: &Shared,
    store: &Arc<dyn DocumentStore>,
    run: BulkRunId,
    write: impl std::future::Future<Output = Result<BulkOutcome>>,
) -> Result<BulkOutcome> {
    let unknown = || Ok(BulkOutcome::Unknown { cancelled: false });
    match tokio::time::timeout(shared.config.bulk_write_timeout, write).await {
        Err(_) => {
            let limit = shared.config.command_timeout;
            if bounded(limit, store.cancel_bulk(run)).await.is_err() {
                log::warn!("bulk write: cancelling the write that outran its bound failed");
            }
            unknown()
        }
        Ok(Err(CoreError::ConnectionInterrupted(_))) => unknown(),
        Ok(other) => other,
    }
}
