//! Pure handlers for the per-stage counts of the Aggregate tab. A count is a read: it is sent on
//! read-only and production sessions alike, never needs a write confirmation, and its failures
//! stay on the stage's card, never a banner; only a server found gone also fails the session, as
//! any read's loss does. One count is out per session at a time: the next stage is sent when the
//! previous answer (or failure) arrives, and an answer that is not for the request awaited is
//! ignored. Nothing here logs a pipeline or a count.
use super::aggregate_actions::target;
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::aggregate_counts::{CountSend, CountsUi};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// What the person does with the stage counts of the active session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountsAction {
    /// Count every enabled stage, one after another.
    Start,
    /// Stop after the count in flight; the numbers so far stay.
    Cancel,
}

/// `state` holding `counts` in the active session's draft. The draft is edited in place, never
/// copied: it may hold a whole preview page.
fn with_counts(mut state: AppState, counts: CountsUi) -> AppState {
    state.aggregate.counts = counts;
    state
}

pub(super) fn on_action(mut state: AppState, action: CountsAction) -> Out {
    // Moved out of the state and put back changed: the results are not copied.
    let ui = std::mem::take(&mut state.aggregate.counts);
    match action {
        CountsAction::Start => start(state, ui),
        CountsAction::Cancel => (with_counts(state, ui.cancelled()), Vec::new()),
    }
}

/// Sends the first count when the active session is connected with a collection selected, no
/// sequence runs and the draft has a stage to count; otherwise nothing changes (the button is off
/// with its reason in those cases, and a double click lands here with a sequence running).
fn start(state: AppState, ui: CountsUi) -> Out {
    let draft = &state.aggregate;
    let plan = crate::viewmodel::aggregate_counts_plan::count_plan(draft, &draft.stages);
    let (Some(target), Ok(plan), false) = (target(&state), plan, ui.is_running()) else {
        return (with_counts(state, ui), Vec::new());
    };
    let (ui, send) = ui.begun(plan, &state.aggregate.stages, &state.aggregate.keys);
    sent(with_counts(state, ui), Some(target), send)
}

/// `state` with `send` as the command for `target`. A count that cannot be sent (nowhere to send
/// it) ends the sequence instead of leaving it waiting for an answer that cannot come.
fn sent(mut state: AppState, target: Option<(Uuid, Namespace)>, send: Option<CountSend>) -> Out {
    let Some(send) = send else {
        return (state, Vec::new());
    };
    let Some((session, ns)) = target else {
        let ui = std::mem::take(&mut state.aggregate.counts).given_up(send.request);
        return (with_counts(state, ui), Vec::new());
    };
    let command = Command::AggregateCount {
        session,
        request: send.request,
        ns,
        stage_index: send.stage_index,
        pipeline: send.pipeline,
    };
    (state, vec![command])
}

/// `state` with the active session's counts ended, results kept: used when another session is
/// brought into view.
pub(super) fn cancelled(mut state: AppState) -> AppState {
    let ui = std::mem::take(&mut state.aggregate.counts);
    with_counts(state, ui.cancelled())
}

/// `state` with `id` active: the session in view loses its running counts when it is left.
pub(super) fn switched_to(state: AppState, id: Uuid) -> AppState {
    match state.active == Some(id) {
        true => state,
        false => cancelled(state).activated(id),
    }
}

/// Whether `ev` answers a count: an `AggregateCounted`, or a count that could not run
/// (`RequestFailed` with context `aggregate_count`).
pub(super) fn is_counts_event(ev: &Event) -> bool {
    matches!(
        ev,
        Event::AggregateCounted { .. }
            | Event::RequestFailed {
                context: "aggregate_count",
                ..
            }
    )
}

/// A count event of its session, active or not (`with_session` makes the event's own session the
/// active one while it applies, so an inactive session's parked draft is the one updated). A
/// closed session or one that is not connected any more drops the late answer. A count that is
/// still awaited and found the server gone also fails the session (the connection-lost path).
pub(super) fn on_event(state: AppState, ev: Event) -> Out {
    let Some(session) = ev.session() else {
        return (state, Vec::new());
    };
    let lost = lost_server(&state, &ev);
    let (state, commands) = state.with_session(session, |s| match (&s.status, ev) {
        (
            ConnStatus::Connected,
            Event::AggregateCounted {
                request,
                ns,
                stage_index,
                outcome,
                ..
            },
        ) => answered(s, request, Some(&ns), Some(stage_index), outcome),
        (ConnStatus::Connected, Event::RequestFailed { request, error, .. }) => {
            failed(s, request, error)
        }
        _ => (s, Vec::new()),
    });
    let Some(error) = lost else {
        return (state, commands);
    };
    let failed = Event::Failed {
        session,
        context: "aggregate_count",
        error,
    };
    let (state, more) = super::reducer::update(state, super::reducer::Input::Event(failed));
    (state, commands.into_iter().chain(more).collect())
}

/// The error of a count failure that says the server is gone, for the request its connected
/// session still awaits; `None` for anything else (a late echo of an older request included).
fn lost_server(state: &AppState, ev: &Event) -> Option<CoreError> {
    let Event::RequestFailed {
        session,
        request,
        error: error @ (CoreError::Unreachable | CoreError::TlsHandshake),
        ..
    } = ev
    else {
        return None;
    };
    let draft = match state.active == Some(*session) {
        true => Some(&*state.aggregate),
        false => state.parked.get(session).map(|v| &*v.aggregate),
    };
    let awaited = draft.is_some_and(|d| d.counts.awaits(*request));
    let connected = state.status_of(*session) == Some(ConnStatus::Connected);
    (awaited && connected).then(|| error.clone())
}

/// The answer to count `request` (of stage `stage_index`, when the event names it) for `ns`,
/// applied to the active session's draft; the next stage's count goes out with it. An answer for
/// another collection than the one in view is stale, whatever its number.
fn answered(
    mut state: AppState,
    request: u64,
    ns: Option<&Namespace>,
    stage_index: Option<usize>,
    outcome: Result<u64, CoreError>,
) -> Out {
    if ns != state.selected_ns() {
        return (state, Vec::new());
    }
    let ui = std::mem::take(&mut state.aggregate.counts);
    // The answer of a cancelled count frees the slot (and is never a result).
    let (ui, held) = ui.released(request);
    if held.is_some() {
        let state = with_counts(state, ui);
        let target = target(&state);
        return sent(state, target, held);
    }
    let Some(stage_index) = stage_index.or_else(|| ui.awaited_stage(request)) else {
        return (with_counts(state, ui), Vec::new());
    };
    let (ui, send) = ui.answered(request, stage_index, outcome);
    let state = with_counts(state, ui);
    let target = target(&state);
    sent(state, target, send)
}

/// A count the runtime could not run (not connected, a lost connection settled by a ping): a lost
/// or replaced connection ends the sequence (the session fails, or the answer belongs to a
/// connection that is gone); any other error is that stage's failure and the next one goes on.
fn failed(mut state: AppState, request: u64, error: CoreError) -> Out {
    match error {
        CoreError::Superseded | CoreError::Unreachable | CoreError::TlsHandshake => {
            // A superseded count is gone from the server: the start held behind it goes out.
            if error == CoreError::Superseded {
                let (ui, held) = std::mem::take(&mut state.aggregate.counts).released(request);
                if held.is_some() {
                    let state = with_counts(state, ui);
                    let target = target(&state);
                    return sent(state, target, held);
                }
                state.aggregate.counts = ui;
            }
            // Nothing is out for the request any more: it ends the sequence awaiting it, or frees
            // the slot it held (and the start held behind it ends: the connection is gone).
            let ui = std::mem::take(&mut state.aggregate.counts).given_up(request);
            (with_counts(state, ui), Vec::new())
        }
        error => {
            let ns = state.selected_ns().cloned();
            answered(state, request, ns.as_ref(), None, Err(error))
        }
    }
}

#[cfg(test)]
#[path = "aggregate_counts_actions_tests.rs"]
mod tests;
