//! Create index... in the query bar's verdict popup: the index builder opens pre-filled with the
//! suggestion of the verdict on screen and the collation of the query it judged, through
//! `index_write_actions::open_with` (which refuses a read-only connection and leaves production to
//! the builder's own write guard). Nothing is built without the builder's Create.
use super::explain_actions::{VerdictState, shown};
use super::index_write_actions::open_with;
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::IndexBuilderState;
use crate::viewmodel::verdict::badge;

/// Opens the builder for the verdict shown now; nothing for any other state, for no suggestion, or
/// when the badge itself offers none (a good verdict, or a plan that ran on another collection).
pub(super) fn create_index(state: AppState) -> Out {
    open_with(state, |s, ns| {
        let on_screen = shown(s);
        let offered = badge(&on_screen, Some(ns)).and_then(|model| model.suggestion);
        offered?;
        let VerdictState::Ready { query, verdict } = on_screen else {
            return None;
        };
        let index = verdict.suggestion?;
        Some(IndexBuilderState::from_suggested(
            ns.clone(),
            &index,
            query.options.collation.as_ref(),
        ))
    })
}
