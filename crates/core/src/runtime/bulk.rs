//! The bulk commands: counting, sampling, previewing, applying and cancelling a bulk update or
//! delete. Each is a read or a write of the session's store; a write that outruns its bound or
//! loses its connection is an unknown outcome, not a failure, and is never settled by a ping.

use std::sync::Arc;

use super::handlers::{bounded, gate};
use super::session::Session;
use super::{BULK_COUNT_MAX_TIME, Command, Event, SessionId, Shared};
use crate::bulk::{BULK_PREVIEW_DOCS, BulkOutcome};
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Query};

/// Whether `command` is one of the bulk commands handled here.
pub(super) fn is_bulk(command: &Command) -> bool {
    matches!(
        command,
        Command::BulkCount { .. }
            | Command::BulkSample { .. }
            | Command::BulkPreview { .. }
            | Command::BulkUpdate { .. }
            | Command::BulkDelete { .. }
            | Command::BulkCancel { .. }
    )
}

/// Runs a bulk command. An `Err` is a failure of a read (or a superseded session); every other
/// failure is an outcome of the command's own event.
pub(super) async fn run(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    id: SessionId,
    store: Arc<dyn DocumentStore>,
    generation: u64,
    command: Command,
) -> Result<Event> {
    // A command for a replaced connection must not run against the new one's store.
    if !session.is_current(generation) {
        return Err(CoreError::Superseded);
    }
    let event = answer(shared, id, &store, command)
        .await
        .map_err(|e| gate(session, generation, e))?;
    if session.is_current(generation) {
        Ok(event)
    } else {
        Err(CoreError::Superseded)
    }
}

async fn answer(
    shared: &Shared,
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    command: Command,
) -> Result<Event> {
    let limit = shared.config.command_timeout;
    Ok(match command {
        Command::BulkCount {
            ns,
            filter,
            request,
            ..
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
        Command::BulkSample {
            ns,
            filter,
            request,
            ..
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
        Command::BulkPreview {
            ns,
            request,
            request_id,
            ..
        } => Event::BulkPreview {
            session,
            outcome: bounded(limit, store.preview_update(&ns, &request)).await,
            ns,
            request_id,
        },
        Command::BulkUpdate {
            ns, request, run, ..
        } => {
            let write = store.update_many(&ns, &request, run);
            let outcome = write_outcome(shared, write).await;
            Event::BulkFinished {
                session,
                ns,
                run,
                outcome,
            }
        }
        Command::BulkDelete {
            ns, request, run, ..
        } => {
            let write = store.delete_many(&ns, &request, run);
            let outcome = write_outcome(shared, write).await;
            Event::BulkFinished {
                session,
                ns,
                run,
                outcome,
            }
        }
        Command::BulkCancel { run, .. } => Event::BulkCancelResult {
            session,
            run,
            outcome: bounded(limit, store.cancel_bulk(run)).await,
        },
        _ => unreachable!("only bulk commands reach the bulk handler"),
    })
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
