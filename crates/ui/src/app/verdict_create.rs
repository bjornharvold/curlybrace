//! Create index... in the query bar's verdict popup: the index builder opens pre-filled with the
//! suggestion of the verdict on screen and the collation of the query it judged, through
//! `index_write_actions::open_checked` (which refuses a read-only connection and leaves production
//! to the builder's own write guard). Nothing is built without the builder's Create.
use super::explain_actions::{VerdictState, shown};
use super::index_write_actions::open_checked;
use super::reducer::Out;
use super::state::{AppState, Banner};
use crate::viewmodel::IndexBuilderState;
use crate::viewmodel::verdict::offered_suggestion;

/// Opens the builder for the verdict shown now. When there is nothing to offer (the verdict is
/// pending, unavailable, gone or good, or planned on another collection), a banner says so.
pub(super) fn create_index(state: AppState) -> Out {
    let make = |s: &AppState, ns: &curlybrace_core::store::Namespace| {
        let on_screen = shown(s);
        let index = offered_suggestion(&on_screen, ns)?;
        let VerdictState::Ready { query, .. } = &on_screen else {
            return None;
        };
        Some(IndexBuilderState::from_suggested(
            ns.clone(),
            index,
            query.options.collation.as_ref(),
        ))
    };
    open_checked(state, make, Some(Banner::IndexSuggestionGone))
}
