//! Dragging sidebar rows onto a target. A drag carries its own payload; the clipboard is only
//! replaced when a drop is accepted, and then the drop is a Paste like the menu's.
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::{
    CopyClipboard, DropContextKey, DropVerdict, DropVerdictCache, NodeRef, PasteTarget,
    drag_payload, drop_verdict, visible_nodes,
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

    /// [`AppState::drop_verdict`] through the drag's cache: the paste rules run again only when
    /// [`AppState::drop_context_key`] of the target changed since the last frame.
    pub fn drop_verdict_cached(
        &self,
        cache: &mut DropVerdictCache,
        payload: &CopyClipboard,
        target: &PasteTarget,
    ) -> DropVerdict {
        let key = self.drop_context_key(payload, target);
        cache.verdict(target, key, || self.drop_verdict(payload, target))
    }

    /// What a verdict for `target` depends on, read without cloning any name list.
    pub fn drop_context_key(
        &self,
        payload: &CopyClipboard,
        target: &PasteTarget,
    ) -> DropContextKey {
        let open = |session| self.status_of(session) == Some(ConnStatus::Connected);
        let session = target.session();
        let db = match target {
            PasteTarget::Connection { .. } => None,
            PasteTarget::Database { db, .. } => Some(db.as_str()),
            PasteTarget::Collection { ns, .. } => Some(ns.db.as_str()),
        };
        let sidebar = self.sidebar_of(session);
        DropContextKey {
            source_open: open(payload.source_session()),
            target_open: open(session),
            target_read_only: self.session_read_only(session),
            databases: sidebar.map_or(0, |s| s.databases.len()),
            collections: sidebar.and_then(|s| {
                let node = s.databases.iter().find(|d| Some(d.name.as_str()) == db)?;
                node.collections.as_ref().map(Vec::len)
            }),
        }
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
