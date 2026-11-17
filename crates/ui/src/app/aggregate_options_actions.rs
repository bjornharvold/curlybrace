//! Pure handlers for the Options section of the Aggregate tab. A pick applies at once; typed text
//! waits for an Apply (the view sends it on Enter, on focus loss or after a pause, and before any
//! other action of the same frame). Applying a valid form that differs from the applied options
//! previews again; an invalid form sends nothing and the pane says why, keeping the last good
//! preview. Options are reads: they work on read-only and production sessions alike. Nothing
//! here logs an option value.
use super::aggregate_actions::with_draft;
use super::aggregate_preview_send::previewed;
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::AggregateState;
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::aggregate_options_state::Applied;

/// What the person does in the Options section of the active session.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionsAction {
    /// Open or close the section.
    Toggle,
    /// Change one control; a pick applies at once, typed text waits for `Apply`.
    Edit(OptionsEdit),
    /// Apply the form as it stands.
    Apply,
    /// Empty the form and apply it.
    Reset,
}

pub(super) fn on_action(mut state: AppState, action: OptionsAction) -> Out {
    // Moved out and put back changed, never cloned: this runs on every keystroke and the draft
    // may hold a whole preview page.
    let draft = std::mem::take(&mut *state.aggregate);
    match action {
        OptionsAction::Toggle => (with_draft(state, draft.options_toggled()), Vec::new()),
        OptionsAction::Edit(edit) => {
            let applies = !edit.is_typing();
            let draft = draft.options_edited(edit);
            match applies {
                true => applied(state, draft.options_applied()),
                false => (with_draft(state, draft), Vec::new()),
            }
        }
        OptionsAction::Apply => applied(state, draft.options_applied()),
        OptionsAction::Reset => applied(state, draft.options_reset()),
    }
}

/// What applying the form leads to: a new preview when the options changed or the block was
/// lifted, the block shown when the form has a problem, nothing otherwise.
fn applied(state: AppState, (draft, outcome): (AggregateState, Applied)) -> Out {
    match outcome {
        Applied::Changed | Applied::Unblocked => previewed(state, draft),
        Applied::Blocked => (with_draft(state, draft.preview_blocked()), Vec::new()),
        Applied::Unchanged => (with_draft(state, draft), Vec::new()),
    }
}

#[cfg(test)]
#[path = "aggregate_options_actions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "aggregate_options_wiring_tests.rs"]
mod wiring_tests;
