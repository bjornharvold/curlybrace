//! Copy from the documents pane. The clipboard it fills is pasted through the same Paste action
//! and dialog as collections: there is no other way to copy documents.
use super::reducer::Out;
use super::state::{AppState, Banner};
use crate::viewmodel::{DocCopy, clipboard_for};

/// Stores `what` of the shown collection as the clipboard. A copy that cannot be made keeps the
/// old clipboard and says why in a banner; nothing is sent to the runtime.
pub(super) fn copy(state: AppState, what: DocCopy) -> Out {
    let Some(ns) = state
        .selected_ns()
        .cloned()
        .filter(|_| state.page.is_some())
    else {
        return (state, Vec::new());
    };
    let next = match clipboard_for(what, state.session_id(), &ns, &state.page_query) {
        Ok(clipboard) => AppState {
            clipboard: Some(clipboard),
            ..state.without_banner()
        },
        Err(block) => AppState {
            banner: Some(Banner::PasteBlocked(block)),
            banner_session: None,
            ..state
        },
    };
    (next, Vec::new())
}
