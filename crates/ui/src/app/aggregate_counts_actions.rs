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

/// A new `state` holding `counts` in the active session's draft. The draft is moved, not cloned
/// (it may hold a whole preview page, shared by `Arc`): only its `counts` field is replaced.
fn with_counts(state: AppState, counts: CountsUi) -> AppState {
    AppState {
        aggregate: Box::new(state.aggregate.with_counts(counts)),
        ..state
    }
}

/// The counts of the active session's draft, copied (their results are shared by `Arc`).
fn counts_of(state: &AppState) -> CountsUi {
    state.aggregate.counts.clone()
}

pub(super) fn on_action(state: AppState, action: CountsAction) -> Out {
    let ui = counts_of(&state);
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
fn sent(state: AppState, target: Option<(Uuid, Namespace)>, send: Option<CountSend>) -> Out {
    let Some(send) = send else {
        return (state, Vec::new());
    };
    let Some((session, ns)) = target else {
        let ui = counts_of(&state).given_up(send.request);
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
pub(super) fn cancelled(state: AppState) -> AppState {
    let ui = counts_of(&state).cancelled();
    with_counts(state, ui)
}

/// `state` with `id` active: the session in view loses its running counts when it is left. An
/// id that is not an open session changes nothing, and nothing is cancelled for it.
pub(super) fn switched_to(state: AppState, id: Uuid) -> AppState {
    match state.is_open(id) {
        true => left_for(state, id).activated(id),
        false => state,
    }
}

/// `state` with the counts of the session in view ended when `id` is about to replace it (a
/// switch, a second connection opened, the session in view closed); `id` already in view leaves
/// nothing behind. Call it right before the active session changes.
pub(super) fn left_for(state: AppState, id: Uuid) -> AppState {
    match state.active {
        Some(current) if current != id => cancelled(state),
        _ => state,
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
/// session still awaits or a cancelled count of it still out (its held start must not go out
/// into the dead connection); `None` for anything else (a late echo of an older request included).
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
    let awaited = draft.is_some_and(|d| d.counts.awaits(*request) || d.counts.lingers(*request));
    let connected = state.status_of(*session) == Some(ConnStatus::Connected);
    (awaited && connected).then(|| error.clone())
}

/// The answer to count `request` (of stage `stage_index`, when the event names it) for `ns`,
/// applied to the active session's draft; the next stage's count goes out with it. An answer for
/// another collection than the one in view is stale, whatever its number.
fn answered(
    state: AppState,
    request: u64,
    ns: Option<&Namespace>,
    stage_index: Option<usize>,
    outcome: Result<u64, CoreError>,
) -> Out {
    // The answer of a cancelled count frees the slot (and is never a result), whatever collection
    // it was for: one cancelled before a collection change is still out.
    let (state, commands) = released_and_sent(state, request);
    if let Some(commands) = commands {
        return (state, commands);
    }
    if ns != state.selected_ns() {
        return (state, Vec::new());
    }
    let ui = counts_of(&state);
    let Some(stage_index) = stage_index.or_else(|| ui.awaited_stage(request)) else {
        return (state, Vec::new());
    };
    let (ui, send) = ui.answered(request, stage_index, outcome);
    let state = with_counts(state, ui);
    let target = target(&state);
    sent(state, target, send)
}

/// The answer or failure of `request` frees the slot of a cancelled count still out: a start
/// held behind it goes out now (its commands are then `Some`). The state returned has the slot
/// freed when it was that count's.
fn released_and_sent(state: AppState, request: u64) -> (AppState, Option<Vec<Command>>) {
    let (ui, held) = counts_of(&state).released(request);
    let state = with_counts(state, ui);
    match held {
        Some(_) => {
            let target = target(&state);
            let (state, commands) = sent(state, target, held);
            (state, Some(commands))
        }
        None => (state, None),
    }
}

/// A count the runtime could not run (not connected, a lost connection settled by a ping): a lost
/// or replaced connection ends the sequence (the session fails, or the answer belongs to a
/// connection that is gone); any other error is that stage's failure and the next one goes on.
fn failed(state: AppState, request: u64, error: CoreError) -> Out {
    match error {
        CoreError::Superseded | CoreError::Unreachable | CoreError::TlsHandshake => {
            // A superseded count is gone from the server: the start held behind it goes out.
            let state = match error == CoreError::Superseded {
                true => match released_and_sent(state, request) {
                    (state, Some(commands)) => return (state, commands),
                    (state, None) => state,
                },
                false => state,
            };
            // Nothing is out for the request any more: it ends the sequence awaiting it, or frees
            // the slot it held (and the start held behind it ends: the connection is gone).
            let ui = counts_of(&state).given_up(request);
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
