//! Applies the actions of one frame of the Aggregate tab as one: the changes apply in order and at
//! most one preview goes out for all of them.
use super::aggregate_actions::{flushed, on_action, with_draft};
use super::aggregate_actions::AggregateAction;
use super::reducer::Out;
use super::state::AppState;
use curlybrace_core::runtime::Command;

/// Applies the actions of one frame in order. Each change still bumps the request, but only the
/// preview of the request that is current afterwards is sent: the earlier ones would be stale on
/// arrival, and each could run on the server for up to its time limit. When the last change was
/// refused (shown as failed, nothing sent) no preview is sent at all.
pub(super) fn batch(state: AppState, actions: Vec<AggregateAction>) -> Out {
    let (state, commands) =
        actions
            .into_iter()
            .fold((state, Vec::new()), |(state, mut commands), action| {
                let (next, more) = on_action(state, action);
                commands.extend(more);
                (next, commands)
            });
    let current = state.aggregate.request;
    let (kept, unwanted): (Vec<_>, Vec<_>) = commands.into_iter().partition(
        |c| !matches!(c, Command::AggregatePreview { request, .. } if *request != current),
    );
    // A preview sent for an intermediate change was dropped: its slot is free, and the
    // current request, if it waited behind it, goes out in its place.
    let state = unwanted.iter().fold(state, |state, c| match c {
        Command::AggregatePreview { request, .. } => {
            let draft = (*state.aggregate).clone().unsent(*request);
            with_draft(state, draft)
        }
        _ => state,
    });
    let (state, flushed) = flushed(state);
    (state, kept.into_iter().chain(flushed).collect())
}

#[cfg(test)]
#[path = "aggregate_batch_tests.rs"]
mod batch_tests;
