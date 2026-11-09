//! The bulk commands: counting, sampling, previewing, applying and cancelling a bulk update or
//! delete. Each is a read or a write of the session's store; a write that outruns its bound or
//! loses its connection is an unknown outcome, not a failure, and is never settled by a ping.

use std::sync::Arc;

use super::handlers::{bounded, gate};
use super::session::Session;
use super::{BULK_COUNT_MAX_TIME, Command, Event, SessionId, Shared};
use crate::bulk::{BULK_PREVIEW_DOCS, BulkOutcome};
use crate::bulk::{BulkDeleteRequest, BulkRunId, BulkUpdateRequest};
use crate::error::{CoreError, Result};
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
}

pub(super) enum WriteBulk {
    Update {
        ns: Namespace,
        request: BulkUpdateRequest,
        run: BulkRunId,
    },
    Delete {
        ns: Namespace,
        request: BulkDeleteRequest,
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
                ns, request, run, ..
            } => Self::Write(WriteBulk::Update { ns, request, run }),
            Command::BulkDelete {
                ns, request, run, ..
            } => Self::Write(WriteBulk::Delete { ns, request, run }),
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
        // applied, and `send_answer` lets `BulkFinished` through for the same reason.
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
    Ok(match read {
        ReadBulk::Count {
            ns,
            filter,
            request,
        } => {
            let max = BULK_COUNT_MAX_TIME;
            let count = bounded(limit, store.count_matching(&ns, &filter, max)).await?;
            Event::BulkCount {
                session,
                ns,
                request,
                count,
            }
        }
        ReadBulk::Sample {
            ns,
            filter,
            request,
        } => {
            // Straight to the store: the page's own find path records query history.
            let query = Query {
                filter,
                limit: BULK_PREVIEW_DOCS as u64,
                ..Query::default()
            };
            let page = bounded(limit, store.find(&ns, &query)).await?;
            Event::BulkSample {
                session,
                ns,
                request,
                docs: page.docs,
            }
        }
        ReadBulk::Preview {
            ns,
            request,
            request_id,
        } => Event::BulkPreview {
            session,
            outcome: bounded(limit, store.preview_update(&ns, &request)).await,
            ns,
            request_id,
        },
        ReadBulk::Cancel { run } => Event::BulkCancelResult {
            session,
            run,
            outcome: bounded(limit, store.cancel_bulk(run)).await,
        },
    })
}

async fn run_write(
    shared: &Shared,
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    write: WriteBulk,
) -> Event {
    let (ns, run, outcome) = match write {
        WriteBulk::Update { ns, request, run } => {
            let outcome = write_outcome(shared, store.update_many(&ns, &request, run)).await;
            (ns, run, outcome)
        }
        WriteBulk::Delete { ns, request, run } => {
            let outcome = write_outcome(shared, store.delete_many(&ns, &request, run)).await;
            (ns, run, outcome)
        }
    };
    Event::BulkFinished {
        session,
        ns,
        run,
        outcome,
    }
}

/// A write's outcome: outrunning the bound or losing the connection leaves the result unknown.
async fn write_outcome(
    shared: &Shared,
    write: impl std::future::Future<Output = Result<BulkOutcome>>,
) -> Result<BulkOutcome> {
    let unknown = || Ok(BulkOutcome::Unknown { cancelled: false });
    match tokio::time::timeout(shared.config.bulk_write_timeout, write).await {
        Err(_) => unknown(),
        Ok(Err(CoreError::ConnectionInterrupted(_))) => unknown(),
        Ok(other) => other,
    }
}
