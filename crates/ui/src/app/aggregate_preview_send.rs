//! Sending the Aggregate tab's preview: which session and collection it goes to, the numbered
//! command with the draft's options, and the preview that waited for the slot. A draft whose
//! options form has a problem sends nothing: the pane says why and keeps the last good preview.
use super::aggregate_actions::with_draft;
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::AggregateState;
use bson::Document;
use curlybrace_core::pipeline::PipelineOptions;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// Keeps `draft` and, when the active session is connected with a collection selected, starts
/// its preview. A pipeline the preview refuses is shown as failed and sends nothing; so is one
/// whose options form has a problem. Without a session, a connection or a collection there is
/// nothing to preview against and the draft is only kept.
pub(super) fn previewed(state: AppState, draft: AggregateState) -> Out {
    if draft.options_blocked() {
        return (with_draft(state, draft.preview_blocked()), Vec::new());
    }
    let Some((session, ns)) = target(&state) else {
        return (with_draft(state, draft), Vec::new());
    };
    let (draft, plan) = draft.begin_preview();
    let (draft, send) = match plan {
        Some((request, pipeline)) => draft.offered(request, ns.clone(), pipeline),
        None => (draft, None),
    };
    let options = draft.options.clone();
    let commands: Vec<Command> = send
        .map(|(request, pipeline)| preview_command(session, request, ns, pipeline, options))
        .into_iter()
        .collect();
    let state = with_draft(state, draft);
    // A preview is a read the person asked for by editing: a dormant tab that sends one is no
    // longer waiting for Run.
    let state = match commands.is_empty() {
        true => state,
        false => state.awake_current(),
    };
    (state, commands)
}

/// The session and collection a preview can be sent for now: the active session when it is
/// connected with a collection selected.
pub(super) fn target(state: &AppState) -> Option<(Uuid, Namespace)> {
    match (state.active, &state.status, state.selected_ns()) {
        (Some(session), ConnStatus::Connected, Some(ns)) => Some((session, ns.clone())),
        _ => None,
    }
}

/// Sends the preview that waited for the slot, when one is free and it still applies. A form
/// with a problem drops it instead (see `AggregateState::preview_blocked`).
pub(super) fn flushed(mut state: AppState) -> Out {
    let target = target(&state);
    let ns = target.as_ref().map(|(_, ns)| ns);
    // Moved out and put back by `sent`: the draft may hold a whole preview page.
    let draft = std::mem::take(&mut *state.aggregate);
    let draft = match draft.options_blocked() && draft.queued.is_some() {
        true => draft.preview_blocked(),
        false => draft,
    };
    let (draft, send) = draft.flushed(ns);
    sent(state, draft, target, send)
}

/// `state` holding `draft`, with `send` as the command for `target` (none without a target).
pub(super) fn sent(
    state: AppState,
    draft: AggregateState,
    target: Option<(Uuid, Namespace)>,
    send: Option<(u64, Vec<Document>)>,
) -> Out {
    let options = draft.options.clone();
    let command = send
        .zip(target)
        .map(|((request, pipeline), (session, ns))| {
            preview_command(session, request, ns, pipeline, options)
        });
    (with_draft(state, draft), command.into_iter().collect())
}

fn preview_command(
    session: Uuid,
    request: u64,
    ns: Namespace,
    pipeline: Vec<Document>,
    options: PipelineOptions,
) -> Command {
    Command::AggregatePreview {
        session,
        request,
        ns,
        pipeline,
        options,
    }
}
