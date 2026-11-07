//! Reducer arms of the Queries view: reading it, the consent step before the profiler is switched
//! (the switch itself goes through the same write guard as every other write) and what the
//! runtime answers. Only `Open`/`Refresh`/`SetRange` (reads) and the two confirmations (the
//! write) ever send a command; asking and cancelling never do.
use super::actions::{gated, mutation};
use super::reducer::Out;
use super::state::{AppState, Banner};
use super::write_guard::{Guard, WriteKind};
use crate::viewmodel::{
    InFlight, IndexesView, PROFILER_LEVEL_OFF, PROFILER_LEVEL_ON, ProfilerConsent, ProfilingView,
    QueriesAction, QueriesState,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::history::{ChartRange, ShapeSummary};
use curlybrace_core::runtime::{Command, ProfilerStatus};
use curlybrace_core::store::{Namespace, ProfilingState};

pub(super) fn on_action(state: AppState, action: QueriesAction) -> Out {
    match action {
        QueriesAction::Open | QueriesAction::Refresh => gated(state, |s| read(s, None)),
        QueriesAction::SetRange(range) => gated(state, |s| read(s, Some(range))),
        QueriesAction::AskEnable => ask(state, PROFILER_LEVEL_ON),
        QueriesAction::AskDisable => ask(state, PROFILER_LEVEL_OFF),
        QueriesAction::CancelEnable => cancel(state),
        QueriesAction::ConfirmEnable => confirm(state, PROFILER_LEVEL_ON),
        QueriesAction::ConfirmDisable => confirm(state, PROFILER_LEVEL_OFF),
    }
}

/// Shows `view`. The Queries view reads its data; the Indexes view has it already.
pub(super) fn show_view(state: AppState, view: IndexesView) -> Out {
    match view {
        IndexesView::Queries => on_action(state, QueriesAction::Open),
        IndexesView::Indexes => {
            let indexes = state.indexes.with_view(IndexesView::Indexes);
            (AppState { indexes, ..state }, Vec::new())
        }
    }
}

/// The indexes state held for `ns`; another collection starts over.
fn held_for(state: &AppState, ns: &Namespace) -> crate::viewmodel::IndexesState {
    match state.indexes.ns.as_ref() == Some(ns) {
        true => state.indexes.clone(),
        false => state.indexes.for_collection(ns.clone()),
    }
}

/// Shows the Queries view and asks for the recorded shapes (under `range`, when the person just
/// chose one) and the profiler state of the collection's database. A read: allowed on a read-only
/// connection. The runtime's profiler answers carry no number, so at most one profiler command is
/// in flight: the state is read only when none is (a read already on its way, or a change being
/// sent or warned about, makes the new one wait for that answer instead).
fn read(state: AppState, range: Option<ChartRange>) -> Out {
    let Some(ns) = state.selected_ns().cloned() else {
        return (state, Vec::new());
    };
    let session = state.session_id();
    let base = held_for(&state, &ns);
    let queries = match range {
        Some(range) => base.queries.clone().with_range(range),
        None => base.queries.clone(),
    };
    let queries = queries.requested();
    let mut commands = vec![Command::QueryShapes {
        session,
        ns: ns.clone(),
        range: queries.range,
        request: queries.request,
    }];
    let sends = queries.in_flight.is_none() && state.write_warning.is_none();
    let waits = matches!(queries.in_flight, Some(InFlight::Read(_)));
    let queries = match (sends, waits || queries.profiling == ProfilingView::Unknown) {
        (true, _) => {
            commands.push(Command::ProfilingState {
                session,
                db: ns.db.clone(),
            });
            queries.read_sent(&ns.db)
        }
        (false, true) => queries.with_profiling(ProfilingView::Loading),
        (false, false) => queries,
    };
    let indexes = base.with_view(IndexesView::Queries).with_queries(queries);
    (AppState { indexes, ..state }, commands)
}

/// Opens the consent step for `level`. Sends nothing. Refused on a read-only connection (the
/// read-only banner, no prompt), and while a profiler command is in flight or the state is still
/// loading, since what is being asked is not known yet.
fn ask(state: AppState, level: i32) -> Out {
    let Some(ns) = state.selected_ns().cloned() else {
        return (state, Vec::new());
    };
    if state.active_is_read_only() {
        let banner = Some(Banner::ReadOnly);
        return (AppState { banner, ..state }, Vec::new());
    }
    let queries = &state.indexes.queries;
    if queries.busy() || state.write_warning.is_some() {
        return (state, Vec::new());
    }
    let base = held_for(&state, &ns);
    let consent = Some(ProfilerConsent { db: ns.db, level });
    let queries = base.queries.clone().with_consent(consent);
    let indexes = base.with_queries(queries);
    (AppState { indexes, ..state }, Vec::new())
}

/// Closes the consent step. Sends nothing; refused while the change is being sent or warned about,
/// like the drop confirmation.
fn cancel(state: AppState) -> Out {
    if state.indexes.queries.writing() || state.write_warning.is_some() {
        return (state, Vec::new());
    }
    let queries = state.indexes.queries.clone().with_consent(None);
    let indexes = state.indexes.clone().with_queries(queries);
    (AppState { indexes, ..state }, Vec::new())
}

/// Sends `SetProfilingLevel` for the database of the open consent step, which must be the one
/// asked for `level` and the one of the collection on screen. Through the write guard: refused on
/// a read-only connection, parked for the production warning on a production one. The prompt
/// stays open until the change is answered. Nothing is sent while another profiler command is in
/// flight or the state is loading.
fn confirm(state: AppState, level: i32) -> Out {
    let db = state
        .indexes
        .queries
        .consent
        .as_ref()
        .filter(|c| c.level == level)
        .map(|c| c.db.clone())
        .filter(|db| state.selected_ns().is_some_and(|ns| ns.db == *db));
    let Some(db) = db else {
        return (state, Vec::new());
    };
    if state.indexes.queries.busy() {
        return (state, Vec::new());
    }
    mutation(
        state,
        None,
        WriteKind::SetProfilingLevel,
        Guard::Warn,
        |session, _| Command::SetProfilingLevel { session, db, level },
    )
}

/// A profiler change was just sent (immediately, or confirmed after the production warning).
pub(super) fn write_sent(state: AppState, db: &str) -> AppState {
    let queries = state.indexes.queries.clone().write_sent(db);
    let indexes = state.indexes.clone().with_queries(queries);
    AppState { indexes, ..state }
}

/// The answer to `ProfilingState` or `SetProfilingLevel` for `db`. With one profiler command in
/// flight, the answer is attributed by it: the answer to the change settles it wherever the person
/// is by now; any other answer is just stored when it is for the collection on screen. A failure
/// is never a banner: a failed read leaves the profiler unavailable (inline, nothing more is
/// sent), and a failed change is ambiguous (the server may have applied it), so the state is read
/// once more.
pub(super) fn profiling_answered(
    state: AppState,
    db: String,
    outcome: Result<ProfilingState, CoreError>,
) -> Out {
    let wrote = state.indexes.queries.in_flight == Some(InFlight::Write(db.clone()));
    let read = state.indexes.queries.in_flight == Some(InFlight::Read(db.clone()));
    let state = match (wrote, read) {
        (false, false) => state,
        _ => settled(state, &db, wrote),
    };
    if state.selected_ns().is_none_or(|ns| ns.db != db) {
        return follow_up(state);
    }
    match outcome {
        Ok(profiling) => (profiling_known(state, profiling, wrote), Vec::new()),
        Err(error) if wrote => reread(state, db, error),
        Err(error) => (
            profiling_set(state, ProfilingView::Unavailable(error)),
            Vec::new(),
        ),
    }
}

/// A profiler command failed as a command (not an outcome): it is treated as the `Err` outcome of
/// the command in flight of that kind, never as a banner. Nothing in flight, nothing to settle.
pub(super) fn command_failed(state: AppState, context: &str, error: CoreError) -> Out {
    let db = match (&state.indexes.queries.in_flight, context) {
        (Some(InFlight::Write(db)), "set_profiling_level") => db.clone(),
        (Some(InFlight::Read(db)), "profiling_state") => db.clone(),
        _ => return (state, Vec::new()),
    };
    profiling_answered(state, db, Err(error))
}

/// The command in flight was answered: nothing is in flight any more, and a change also ends the
/// pending marker the write guard set for it.
fn settled(state: AppState, db: &str, wrote: bool) -> AppState {
    let pending = match wrote {
        true => state.pending.filter(|p| p.ns.db != db),
        false => state.pending,
    };
    let queries = state.indexes.queries.clone().settled();
    let indexes = state.indexes.clone().with_queries(queries);
    AppState {
        indexes,
        pending,
        ..state
    }
}

/// After an answer for another database: the collection on screen may have been waiting for it
/// (it asked while that command was in flight), so its state is read now.
fn follow_up(state: AppState) -> Out {
    let queries = &state.indexes.queries;
    let waiting = queries.in_flight.is_none()
        && queries.profiling == ProfilingView::Loading
        && state.write_warning.is_none();
    let Some(ns) = state.selected_ns().cloned().filter(|_| waiting) else {
        return (state, Vec::new());
    };
    let command = Command::ProfilingState {
        session: state.session_id(),
        db: ns.db.clone(),
    };
    let queries = queries.clone().read_sent(&ns.db);
    let indexes = state.indexes.clone().with_queries(queries);
    (AppState { indexes, ..state }, vec![command])
}

fn profiling_set(state: AppState, profiling: ProfilingView) -> AppState {
    let queries = state.indexes.queries.clone().with_profiling(profiling);
    let indexes = state.indexes.clone().with_queries(queries);
    AppState { indexes, ..state }
}

/// The profiler state is known. The consent step closes when it was answered by the change it
/// asked for, or when the state already is what it asked for.
fn profiling_known(state: AppState, profiling: ProfilingState, wrote: bool) -> AppState {
    let queries = &state.indexes.queries;
    let consent = queries
        .consent
        .clone()
        .filter(|c| !wrote && c.level != profiling.level);
    let queries = QueriesState {
        profiling: ProfilingView::Known(profiling),
        consent,
        error: None,
        ..queries.clone()
    };
    let indexes = state.indexes.clone().with_queries(queries);
    AppState { indexes, ..state }
}

/// The outcome of the change is unknown: keep the error, wait for the state and ask for it once.
/// If that read fails too the state is unavailable and nothing more is sent.
fn reread(state: AppState, db: String, error: CoreError) -> Out {
    let session = state.session_id();
    let queries = QueriesState {
        error: Some(error),
        ..state.indexes.queries.clone()
    }
    .read_sent(&db);
    let indexes = state.indexes.clone().with_queries(queries);
    let command = Command::ProfilingState { session, db };
    (AppState { indexes, ..state }, vec![command])
}

/// The answer to `QueryShapes`; one for another collection or range, or to a request that is not
/// the newest one sent, is stale.
pub(super) fn shapes_arrived(
    state: AppState,
    ns: Namespace,
    range: ChartRange,
    shapes: Vec<ShapeSummary>,
    request: u64,
) -> AppState {
    let queries = &state.indexes.queries;
    if state.indexes.ns.as_ref() != Some(&ns)
        || queries.range != range
        || queries.request != request
    {
        return state;
    }
    let queries = queries.clone().with_shapes(shapes);
    let indexes = state.indexes.clone().with_queries(queries);
    AppState { indexes, ..state }
}

/// The read of the shapes failed: only the awaited request settles, and the error is shown inline.
pub(super) fn shapes_failed(state: AppState, request: u64, error: CoreError) -> AppState {
    if state.indexes.queries.request != request {
        return state;
    }
    let queries = state.indexes.queries.clone().failed(error);
    let indexes = state.indexes.clone().with_queries(queries);
    AppState { indexes, ..state }
}

/// The session's profiler started, paused or lost entries. Runs while the event's own session is
/// the active one (see `AppState::with_session`).
pub(super) fn profiler_reported(state: AppState, status: ProfilerStatus) -> AppState {
    let queries = state.indexes.queries.clone().with_status(status);
    let indexes = state.indexes.clone().with_queries(queries);
    AppState { indexes, ..state }
}
