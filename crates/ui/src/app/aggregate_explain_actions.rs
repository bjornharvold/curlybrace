//! Pure handlers for the Explain section of the Aggregate tab. An explain is a read: it is sent
//! on read-only and production sessions alike, never needs a write confirmation, and its
//! failures stay in the Plan tab, never a banner; only a server found gone also fails the
//! session, as any read's loss does. At most one explain is out per session: Explain does
//! nothing while one waits, and an answer that is not for the newest request number is ignored.
use super::aggregate_actions::target;
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::aggregate_explain::{ExplainUi, ResultTab};
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::ExplainVerbosity;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Explain, Namespace};

/// What the person does in the Explain section of the active session.
#[derive(Clone, Debug, PartialEq)]
pub enum ExplainAction {
    /// Ask the server for the plan of the enabled stages up to the selected one.
    Run,
    /// Plan only, or run the pipeline for counters, on the next Run.
    SetVerbosity(ExplainVerbosity),
    SelectTab(ResultTab),
    /// The view put the explain document on the clipboard; nothing changes.
    CopyRaw,
}

/// `state` holding `explain` in the active session's draft. The draft is edited in place, never
/// copied: it may hold a whole preview page.
fn with_explain(mut state: AppState, explain: ExplainUi) -> AppState {
    state.aggregate.explain = explain;
    state
}

pub(super) fn on_action(mut state: AppState, action: ExplainAction) -> Out {
    // Moved out of the state and put back changed: the plan is not copied.
    let ui = std::mem::take(&mut state.aggregate.explain);
    match action {
        ExplainAction::Run => run(state, ui),
        ExplainAction::SetVerbosity(verbosity) => (
            with_explain(state, ui.with_verbosity(verbosity)),
            Vec::new(),
        ),
        ExplainAction::SelectTab(tab) => (with_explain(state, ui.with_tab(tab)), Vec::new()),
        ExplainAction::CopyRaw => (with_explain(state, ui), Vec::new()),
    }
}

/// Sends the explain when the active session is connected with a collection selected, nothing
/// waits for an answer and the draft has something to explain; otherwise nothing changes (the
/// button is off with its reason in those cases).
fn run(state: AppState, ui: ExplainUi) -> Out {
    let request = crate::viewmodel::aggregate_explain_block::explain_request(
        &state.aggregate,
        &state.aggregate.stages,
    );
    let (Some((session, ns)), Ok(stages), false) = (target(&state), request, ui.is_loading())
    else {
        return (with_explain(state, ui), Vec::new());
    };
    let (ui, request, verbosity) = ui.begun(stages.clone());
    let command = Command::AggregateExplain {
        session,
        request,
        ns,
        stages,
        verbosity,
        // 4b: the draft's options.
        options: curlybrace_core::pipeline::PipelineOptions::default(),
    };
    (with_explain(state, ui), vec![command])
}

/// Whether `ev` answers an explain: an `AggregateExplained`, or an explain that could not run
/// (`RequestFailed` with context `aggregate_explain`).
pub(super) fn is_explain_event(ev: &Event) -> bool {
    matches!(
        ev,
        Event::AggregateExplained { .. }
            | Event::RequestFailed {
                context: "aggregate_explain",
                ..
            }
    )
}

/// An explain event of its session, active or not (`with_session` makes the event's own session
/// the active one while it applies, so an inactive session's parked draft is the one updated).
/// A closed session or one that is not connected any more drops the late answer. An explain that
/// is still awaited and found the server gone also fails the session (the connection-lost path).
pub(super) fn on_event(state: AppState, ev: Event) -> Out {
    let Some(session) = ev.session() else {
        return (state, Vec::new());
    };
    let lost = lost_server(&state, &ev);
    let (state, commands) = state.with_session(session, |s| match (&s.status, ev) {
        (
            ConnStatus::Connected,
            Event::AggregateExplained {
                request,
                ns,
                outcome,
                ..
            },
        ) => answered(s, request, Some(&ns), outcome),
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
        context: "aggregate_explain",
        error,
    };
    let (state, more) = super::reducer::update(state, super::reducer::Input::Event(failed));
    (state, commands.into_iter().chain(more).collect())
}

/// The error of an explain failure that says the server is gone, for the request its connected
/// session still awaits; `None` for anything else (a late echo of a replaced request included).
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
    let awaited = draft.is_some_and(|d| d.explain.awaits(*request));
    let connected = state.status_of(*session) == Some(ConnStatus::Connected);
    (awaited && connected).then(|| error.clone())
}

/// The answer to explain `request` for `ns`, applied to the active session's draft. An answer for
/// another collection than the one in view is stale, whatever its number.
fn answered(
    mut state: AppState,
    request: u64,
    ns: Option<&Namespace>,
    outcome: Result<Explain, CoreError>,
) -> Out {
    if ns != state.selected_ns() {
        return (state, Vec::new());
    }
    let ui = std::mem::take(&mut state.aggregate.explain);
    let next = ui.arrived(request, outcome);
    (with_explain(state, next), Vec::new())
}

/// An explain the runtime could not run (not connected, a lost connection settled by a ping): the
/// same inline failure as an `Err` outcome. A superseded one belongs to a connection that was
/// replaced: it shows nothing and only frees the pane.
fn failed(mut state: AppState, request: u64, error: CoreError) -> Out {
    match error {
        CoreError::Superseded => {
            let ui = std::mem::take(&mut state.aggregate.explain).given_up(request);
            (with_explain(state, ui), Vec::new())
        }
        error => {
            let ns = state.selected_ns().cloned();
            answered(state, request, ns.as_ref(), Err(error))
        }
    }
}

#[cfg(test)]
#[path = "aggregate_explain_actions_tests.rs"]
mod tests;
