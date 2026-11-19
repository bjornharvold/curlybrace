//! Pure handlers for the Aggregate tab: the person's edits of the pipeline draft and the
//! runtime's preview answers. A preview is a read: it is sent on read-only and production
//! sessions alike, with no write confirmation, and its failures stay in the preview pane; only a
//! server found gone also fails the session, as any read's loss does.
pub(super) use super::aggregate_preview_send::{flushed, target};
use super::aggregate_preview_send::{previewed, sent};
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::AggregateState;
use crate::viewmodel::aggregate::Mode;
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
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
    /// The actions of one frame, applied in order; at most one preview is sent for all of them
    /// (see [`batch`]).
    Batch(Vec<AggregateAction>),
    /// Save was clicked: opens the name dialog.
    SaveClicked,
    SaveNameEdited(String),
    /// Save the draft under the dialog's name (asks to replace when the name is taken).
    SaveConfirmed,
    /// A saved pipeline was picked in the Open menu (asks first when the draft is modified).
    OpenMenuPicked(Uuid),
    /// Replace the draft with the pipeline the open prompt names.
    OpenConfirmed,
    RenameStarted(Uuid),
    RenameEdited(String),
    RenameConfirmed,
    DeleteAsked(Uuid),
    DeleteConfirmed,
    /// Close whichever saved-pipeline dialog is open, changing nothing.
    DialogCancelled,
    /// Run was clicked: opens the typed confirmation of the pipeline's `$out` / `$merge`.
    RunClicked,
    /// The confirmation text changed.
    RunTyped(String),
    /// Confirm was pressed: runs the pipeline when the typed text names the target.
    RunConfirmed,
    /// The confirmation dialog was closed without running.
    RunDismissed,
    /// The Copy as code dialog: opening it for the enabled stages, or one of its own actions.
    /// Carried here, not as an `Input` of its own, because every arm of the reducer's `update`
    /// costs it a copy of the whole state on the stack, which is near the debug-build limit.
    Code(super::CodeAction),
    /// The Explain section: running it, its verbosity and tab. Carried here for the same reason
    /// as `Code`.
    Explain(super::ExplainAction),
    /// The per-stage counts: Count stages and Cancel. Carried here for the same reason.
    Counts(super::CountsAction),
    /// The Options section: its toggle, edits, Apply and Reset. Carried here for the same reason.
    Options(super::OptionsAction),
}

impl AggregateAction {
    /// Whether this is one of the Save / Open / Rename / Delete actions of the saved pipelines.
    /// Only a shortcut for `on_action`: its match still names every variant.
    pub(super) fn is_saved_pipeline_action(&self) -> bool {
        matches!(
            self,
            Self::SaveClicked
                | Self::SaveNameEdited(_)
                | Self::SaveConfirmed
                | Self::OpenMenuPicked(_)
                | Self::OpenConfirmed
                | Self::RenameStarted(_)
                | Self::RenameEdited(_)
                | Self::RenameConfirmed
                | Self::DeleteAsked(_)
                | Self::DeleteConfirmed
                | Self::DialogCancelled
        )
    }

    /// The actions of one frame as one action: none is nothing, one is itself, more are a
    /// `Batch`.
    pub fn batched(mut actions: Vec<AggregateAction>) -> Option<AggregateAction> {
        match actions.len() {
            0 => None,
            1 => actions.pop(),
            _ => Some(AggregateAction::Batch(actions)),
        }
    }
}

/// Applies `action` to the active session's draft. Every change of the pipeline or of the
/// selection previews the selected stage at once.
pub(super) fn on_action(state: AppState, action: AggregateAction) -> Out {
    if action.is_saved_pipeline_action() {
        // Before the draft is copied for the others: a name typed into a dialog changes the
        // dialog only, and the saved handler moves the draft rather than copy it.
        return super::aggregate_saved_actions::on_action(state, action);
    }
    // Also before the draft is copied: the Copy as code dialog only reads it.
    let action = match action {
        AggregateAction::Code(code) => return super::code_export_actions::on_action(state, code),
        // And the explain moves its own part of the draft out and back, never a copy of it.
        AggregateAction::Explain(explain) => {
            return super::aggregate_explain_actions::on_action(state, explain);
        }
        AggregateAction::Counts(counts) => {
            return super::aggregate_counts_actions::on_action(state, counts);
        }
        // The options form moves the draft out and back too: a keystroke copies nothing.
        AggregateAction::Options(options) => {
            return super::aggregate_options_actions::on_action(state, options);
        }
        other => other,
    };
    let draft = (*state.aggregate).clone();
    match action {
        // Handled above; listed so the match stays exhaustive and a new variant must be placed.
        AggregateAction::Code(code) => super::code_export_actions::on_action(state, code),
        AggregateAction::Explain(explain) => {
            super::aggregate_explain_actions::on_action(state, explain)
        }
        AggregateAction::Counts(counts) => {
            super::aggregate_counts_actions::on_action(state, counts)
        }
        AggregateAction::Options(options) => {
            super::aggregate_options_actions::on_action(state, options)
        }
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
        AggregateAction::Batch(actions) => super::aggregate_batch::batch(state, actions),
        // Listed one by one, so a new variant must be placed here and `Batch` can never fall
        // through to the saved-pipelines handler.
        run @ (AggregateAction::RunClicked
        | AggregateAction::RunTyped(_)
        | AggregateAction::RunConfirmed
        | AggregateAction::RunDismissed) => super::aggregate_run_actions::on_action(state, run),
        saved @ (AggregateAction::SaveClicked
        | AggregateAction::SaveNameEdited(_)
        | AggregateAction::SaveConfirmed
        | AggregateAction::OpenMenuPicked(_)
        | AggregateAction::OpenConfirmed
        | AggregateAction::RenameStarted(_)
        | AggregateAction::RenameEdited(_)
        | AggregateAction::RenameConfirmed
        | AggregateAction::DeleteAsked(_)
        | AggregateAction::DeleteConfirmed
        | AggregateAction::DialogCancelled) => {
            super::aggregate_saved_actions::on_action(state, saved)
        }
    }
}

/// `state` holding `draft` as the active session's Aggregate draft.
pub(super) fn with_draft(state: AppState, draft: AggregateState) -> AppState {
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

/// Text mode starts from the stages; leaving it with edited text previews the parsed stages,
/// leaving it unchanged keeps the stages and sends nothing, and invalid text stays in text mode
/// with its error and sends nothing. Asking for the mode already shown keeps the draft as it is
/// (the text buffer is not overwritten, the stages are not re-parsed).
fn set_mode(state: AppState, draft: AggregateState, mode: Mode) -> Out {
    if draft.mode == mode {
        return (state, Vec::new());
    }
    match mode {
        Mode::Text => (with_draft(state, draft.to_text()), Vec::new()),
        Mode::Stages => match draft.from_text() {
            next if next.request != state.aggregate.request => previewed(state, next),
            next => (with_draft(state, next), Vec::new()),
        },
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
/// not connected any more drops the late answer. A preview that is still awaited and found the
/// server gone then fails the session like any read (the connection-lost path).
pub(super) fn on_event(state: AppState, ev: Event) -> Out {
    let Some(session) = ev.session() else {
        return (state, Vec::new());
    };
    let lost = lost_server(&state, &ev);
    let owner = super::tab_routing::owner_of(&state, session, &ev);
    let (state, commands) = state.with_session_tab(session, owner, |s| match (&s.status, ev) {
        (
            ConnStatus::Connected,
            Event::AggregatePreviewed {
                request, outcome, ..
            },
        ) => arrived(s, request, outcome, lost.is_none()),
        (ConnStatus::Connected, Event::RequestFailed { request, error, .. }) => {
            request_failed(s, request, error, lost.is_none())
        }
        _ => (s, Vec::new()),
    });
    let Some(error) = lost else {
        return (state, commands);
    };
    let failed = Event::Failed {
        session,
        context: "aggregate_preview",
        error,
    };
    let (state, more) = super::reducer::update(state, super::reducer::Input::Event(failed));
    (state, commands.into_iter().chain(more).collect())
}

/// The error of a preview failure that says the server is gone (`Unreachable`, a failed TLS
/// handshake), for the request its connected session still awaits (the current one or the one
/// in flight, behind which the current one may wait); `None` for anything else. A request that
/// was already replaced and answered is a late echo.
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
    // The draft of the tab the request belongs to (a parked tab's, when it awaits it).
    let draft = super::tab_routing::draft_for(state, *session, ev);
    let awaited = draft.is_some_and(|d| d.request == *request || d.in_flight == Some(*request));
    let connected = state.status_of(*session) == Some(ConnStatus::Connected);
    (awaited && connected).then(|| error.clone())
}

/// The answer to preview `request`, applied to the active session's draft. A stale answer
/// changes nothing; a failure keeps the last good preview and is shown in the preview pane only,
/// never as a banner. Either way the answer of the preview in flight frees the slot, and with
/// `release` the newest waiting preview goes out (not when the session is about to fail).
fn arrived(
    state: AppState,
    request: u64,
    outcome: Result<Vec<Document>, CoreError>,
    release: bool,
) -> Out {
    let current = state.aggregate.request;
    let lost = outcome.clone().err().filter(|_| !release);
    let mut draft = (*state.aggregate).clone().arrived(request, outcome);
    // The session is about to fail on this error, so the waiting preview is dropped and never
    // sent: a pane still loading it would wait for an answer that cannot come (the session may
    // stay connected while an editor is open). It fails with the same connection error instead
    // of going idle, which would ask again at once against the dead connection.
    if let Some(error) = lost {
        draft = draft.arrived(current, Err(error));
    }
    released(with_draft(state, draft), request, release)
}

/// Frees the slot of `request` and, with `release`, sends the waiting preview.
fn released(state: AppState, request: u64, release: bool) -> Out {
    let target = target(&state);
    let ns = target.as_ref().map(|(_, ns)| ns).filter(|_| release);
    let (draft, send) = (*state.aggregate).clone().released(request, ns);
    sent(state, draft, target, send)
}

/// A preview the runtime could not run (not connected, a lost connection settled by a ping, a
/// read-only refusal): the same inline failure as an `Err` outcome, never a banner. A superseded
/// one belongs to a connection that was replaced: it shows nothing but frees its slot.
fn request_failed(state: AppState, request: u64, error: CoreError, release: bool) -> Out {
    match error {
        CoreError::Superseded => released(state, request, release),
        error => arrived(state, request, Err(error), release),
    }
}

#[cfg(test)]
#[path = "aggregate_actions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "aggregate_flight_tests.rs"]
mod flight_tests;
