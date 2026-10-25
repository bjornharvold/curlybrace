//! Dragging sidebar rows onto a target. A drag carries its own payload; the clipboard is only
//! replaced when a drop is accepted, and then the drop is a Paste like the menu's.
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::{
    CopyClipboard, DropVerdict, NodeRef, PasteTarget, drag_payload, drop_verdict, visible_nodes,
};

impl AppState {
    /// The clipboard a drag of `dragged` carries: the whole selection when the row is part of it
    /// (only rows that are on screen), else the row alone. `None`: nothing copyable to drag.
    pub fn drag_payload(&self, dragged: &NodeRef) -> Option<CopyClipboard> {
        let session = dragged.session();
        let visible = self
            .sidebar_of(session)
            .map(|sidebar| visible_nodes(session, sidebar))
            .unwrap_or_default();
        drag_payload(&self.selection, dragged, &visible)
    }

    /// Whether `target` takes `payload` if it is dropped there, and if not, why.
    pub fn drop_verdict(&self, payload: &CopyClipboard, target: &PasteTarget) -> DropVerdict {
        self.with_paste_context(|ctx| drop_verdict(payload, target, ctx))
    }
}

/// A drop of `clipboard` on `target`: when the paste rules take it, it becomes the clipboard and
/// is pasted exactly like the menu does (the paste dialog opens). A target that refuses it is a
/// no-op: no request, no banner, and the clipboard stays as it was.
pub(super) fn drop_onto(state: AppState, clipboard: CopyClipboard, target: PasteTarget) -> Out {
    if state.drop_verdict(&clipboard, &target) != DropVerdict::Accept {
        return (state, Vec::new());
    }
    // A copy under way keeps its dialog: the paste below leaves it alone, so the clipboard too.
    if state.copy.is_some() {
        return (state, Vec::new());
    }
    let state = AppState {
        clipboard: Some(clipboard),
        ..state
    };
    super::copy_actions::paste(state, target)
}
