//! The Plan tab's follow-ups: Try an index and Clear hint set the draft's hint through the
//! options path (so Options shows it, "modified" appears and the preview runs again) and explain
//! again at the verbosity selected now; Create index... opens the existing index builder
//! pre-filled with the suggestion, through `index_write_actions::open_with`, which refuses a
//! read-only connection and leaves production to the builder's own write guard. Nothing is built
//! without that dialog.
use super::aggregate_explain_actions::on_action as explain;
use super::aggregate_options_actions::on_action as options;
use super::index_write_actions::open_with;
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use super::{ExplainAction, OptionsAction};
use crate::viewmodel::IndexBuilderState;
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::aggregate_plan_next::plan_next;

/// Whether the controls that send are off (an explain is out, there is no connection, or the
/// options form has a problem).
fn blocked(state: &AppState) -> bool {
    let connected = state.status == ConnStatus::Connected;
    plan_next(&state.aggregate, state.selected_ns(), connected)
        .blocked
        .is_some()
}

/// Sets the hint to the index `name` and explains again.
pub(super) fn try_index(state: AppState, name: String) -> Out {
    chained(state, OptionsEdit::HintPicked(name))
}

/// Removes the hint and explains again.
pub(super) fn clear_hint(state: AppState) -> Out {
    chained(state, OptionsEdit::HintCleared)
}

/// The options edit (which previews when it changed the options), then the explain: the commands
/// go out in that order. Nothing happens while the controls are off.
fn chained(state: AppState, edit: OptionsEdit) -> Out {
    if blocked(&state) {
        return (state, Vec::new());
    }
    let (state, mut commands) = options(state, OptionsAction::Edit(edit));
    let (state, explained) = explain(state, ExplainAction::Run);
    commands.extend(explained);
    (state, commands)
}

/// Opens the index builder with the suggestion of the plan in view; nothing without one.
pub(super) fn create_index(state: AppState) -> Out {
    open_with(state, |s, ns| {
        let next = plan_next(&s.aggregate, Some(ns), true);
        let index = next.suggested?;
        let collation = s.aggregate.explain.done()?.options.collation.as_ref();
        Some(IndexBuilderState::from_suggested(
            ns.clone(),
            &index,
            collation,
        ))
    })
}

#[cfg(test)]
#[path = "aggregate_plan_actions_tests.rs"]
mod tests;
