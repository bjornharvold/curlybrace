//! The profiler and query-shape commands. The two profiler commands answer with
//! [`Event::ProfilingState`] whatever happens, so a failure, a read-only refusal or a timeout is
//! shown inline and never raises the error banner. Neither starts or stops a background task, and
//! `SetProfilingLevel` is the only place in the runtime that sets a profiler level.

use std::sync::Arc;

use super::indexes::shapes_for;
use super::{Event, SessionId};
use crate::error::{CoreError, Result};
use crate::history::{ChartRange, HistoryDb};
use crate::store::{DocumentStore, Namespace, ProfilingState};

/// The database a profiler command addresses, if it is one.
pub(super) fn profiler_db(command: &super::Command) -> Option<&str> {
    match command {
        super::Command::ProfilingState { db, .. }
        | super::Command::SetProfilingLevel { db, .. } => Some(db),
        _ => None,
    }
}

/// The answer to a profiler command that could not finish: carries `error` as its outcome.
pub(super) fn profiler_failure(session: SessionId, db: String, error: CoreError) -> Event {
    Event::ProfilingState {
        session,
        db,
        outcome: Err(error),
    }
}

fn answer(session: SessionId, db: String, outcome: Result<ProfilingState>) -> Event {
    Event::ProfilingState {
        session,
        db,
        outcome,
    }
}

pub(super) async fn profiling_state(
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    db: String,
) -> Event {
    let outcome = store.profiling_level(&db).await;
    answer(session, db, outcome)
}

/// A read-only store refuses the write itself, so the refusal needs no flag of the caller.
pub(super) async fn set_profiling_level(
    session: SessionId,
    store: &Arc<dyn DocumentStore>,
    db: String,
    level: i32,
) -> Event {
    let outcome = store.set_profiling_level(&db, level).await;
    answer(session, db, outcome)
}

/// The shapes recorded for `ns` over `range`; none when history is off. A failure is the
/// caller's `?`, which becomes an [`Event::RequestFailed`].
pub(super) async fn query_shapes(
    session: SessionId,
    history: Option<Arc<HistoryDb>>,
    ns: Namespace,
    range: ChartRange,
    request: u64,
) -> Result<Event> {
    let shapes = shapes_for(history, session, &ns, range).await?;
    Ok(Event::QueryShapes {
        session,
        ns,
        range,
        shapes,
        request,
    })
}
