//! Pure handlers for the Aggregate tab: the person's edits of the pipeline draft and the
//! runtime's preview answers. A preview is a read: it is sent on read-only and production
//! sessions alike, with no write confirmation, and its failures stay in the preview pane.
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::AggregateState;
use crate::viewmodel::aggregate::Mode;
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// What the person does on the Aggregate tab of the active session.
#[derive(Clone, Debug, PartialEq)]
pub enum AggregateAction {
    /// Append a stage named like this (e.g. `$match`) with its template body.
    AddStage(String),
    Remove(usize),
    MoveUp(usize),
    MoveDown(usize),
    /// Enable or disable a stage.
    Toggle(usize),
    /// Replace the text of a stage.
    Edit(usize, String),
    /// Preview the output of this stage.
    Select(usize),
    SetMode(Mode),
    /// Replace the text-mode buffer; nothing is previewed until the text goes back to stages.
    EditText(String),
    /// Preview the selected stage again.
    Refresh,
}

/// Applies `action` to the active session's draft. Every change of the pipeline or of the
/// selection previews the selected stage at once.
pub(super) fn on_action(state: AppState, action: AggregateAction) -> Out {
    let draft = (*state.aggregate).clone();
    match action {
        AggregateAction::AddStage(name) => changed(state, draft.add_stage(&name)),
        AggregateAction::Remove(i) => changed(state, draft.remove(i)),
        AggregateAction::MoveUp(i) => changed(state, draft.move_up(i)),
        AggregateAction::MoveDown(i) => changed(state, draft.move_down(i)),
        AggregateAction::Toggle(i) => changed(state, draft.toggle(i)),
        AggregateAction::Edit(i, text) => changed(state, draft.edit(i, &text)),
        AggregateAction::Select(i) => changed(state, draft.select(i)),
        AggregateAction::SetMode(mode) => set_mode(state, draft, mode),
        AggregateAction::EditText(text) => (with_draft(state, draft.with_text(&text)), Vec::new()),
        AggregateAction::Refresh => previewed(state, draft),
    }
}

/// `state` holding `draft` as the active session's Aggregate draft.
fn with_draft(state: AppState, draft: AggregateState) -> AppState {
    AppState {
        aggregate: Box::new(draft),
        ..state
    }
}

/// Keeps `next`; previews it only when the op changed something (every change bumps the
/// request number), so a no-op such as moving the first stage up sends nothing.
fn changed(state: AppState, next: AggregateState) -> Out {
    match next.request == state.aggregate.request {
        true => (state, Vec::new()),
        false => previewed(state, next),
    }
}

/// Text mode starts from the stages; leaving it previews the parsed stages, while invalid text
/// stays in text mode with its error and sends nothing. Asking for the mode already shown keeps
/// the draft as it is (the text buffer is not overwritten, the stages are not re-parsed).
fn set_mode(state: AppState, draft: AggregateState, mode: Mode) -> Out {
    if draft.mode == mode {
        return (state, Vec::new());
    }
    match mode {
        Mode::Text => (with_draft(state, draft.to_text()), Vec::new()),
        Mode::Stages => match draft.from_text() {
            next if next.mode == Mode::Stages => previewed(state, next),
            next => (with_draft(state, next), Vec::new()),
        },
    }
}

/// Keeps `draft` and, when the active session is connected with a collection selected, starts
/// its preview. A pipeline the preview refuses is shown as failed and sends nothing; without a
/// session, a connection or a collection there is nothing to preview against and the draft is
/// only kept.
fn previewed(state: AppState, draft: AggregateState) -> Out {
    let target = match (state.active, &state.status, state.selected_ns()) {
        (Some(session), ConnStatus::Connected, Some(ns)) => Some((session, ns.clone())),
        _ => None,
    };
    let Some((session, ns)) = target else {
        return (with_draft(state, draft), Vec::new());
    };
    let (draft, plan) = draft.begin_preview();
    let commands = plan
        .map(|(request, pipeline)| preview_command(session, request, ns, pipeline))
        .into_iter()
        .collect();
    (with_draft(state, draft), commands)
}

fn preview_command(session: Uuid, request: u64, ns: Namespace, pipeline: Vec<Document>) -> Command {
    Command::AggregatePreview {
        session,
        request,
        ns,
        pipeline,
    }
}

/// Whether `ev` answers a preview: an `AggregatePreviewed`, or a preview that could not run
/// (`RequestFailed` with context `aggregate_preview`).
pub(super) fn is_preview_event(ev: &Event) -> bool {
    matches!(
        ev,
        Event::AggregatePreviewed { .. }
            | Event::RequestFailed {
                context: "aggregate_preview",
                ..
            }
    )
}

/// A preview event (see [`is_preview_event`]) of its session, active or not: `with_session`
/// makes the event's own session the active one while it applies, so an inactive session's
/// parked draft is the one updated. A closed session (`with_session` ignores it) or one that is
/// not connected any more drops the late answer.
pub(super) fn on_event(state: AppState, ev: Event) -> Out {
    let Some(session) = ev.session() else {
        return (state, Vec::new());
    };
    state.with_session(session, |s| match (&s.status, ev) {
        (
            ConnStatus::Connected,
            Event::AggregatePreviewed {
                request, outcome, ..
            },
        ) => arrived(s, request, outcome),
        (ConnStatus::Connected, Event::RequestFailed { request, error, .. }) => {
            request_failed(s, request, error)
        }
        _ => (s, Vec::new()),
    })
}

/// The answer to preview `request`, applied to the active session's draft. A stale answer
/// changes nothing; a failure keeps the last good preview and is shown in the preview pane only,
/// never as a banner.
fn arrived(state: AppState, request: u64, outcome: Result<Vec<Document>, CoreError>) -> Out {
    let draft = (*state.aggregate).clone().arrived(request, outcome);
    (with_draft(state, draft), Vec::new())
}

/// A preview the runtime could not run (not connected, a lost connection settled by a ping, a
/// read-only refusal): the same inline failure as an `Err` outcome, never a banner. A superseded
/// one belongs to a connection that was replaced and is consumed.
fn request_failed(state: AppState, request: u64, error: CoreError) -> Out {
    match error {
        CoreError::Superseded => (state, Vec::new()),
        error => arrived(state, request, Err(error)),
    }
}

#[cfg(test)]
#[path = "aggregate_actions_tests.rs"]
mod tests;
