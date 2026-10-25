//! Selecting sidebar rows, Copy and Paste. Pure like the rest of the reducer.
use super::reducer::Out;
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{
    NodeRef, PasteBlock, PasteContext, PasteEntry, PasteTarget, SelectMode, SidebarAction,
    SidebarSelection, SidebarState, clipboard_from_selection, copy_entry, paste_entry,
    visible_nodes,
};
use uuid::Uuid;

impl AppState {
    /// The sidebar tree of the open session `id`, wherever its view lives.
    pub fn sidebar_of(&self, id: Uuid) -> Option<&SidebarState> {
        if !self.is_open(id) {
            return None;
        }
        match self.active == Some(id) {
            true => Some(&self.sidebar),
            false => self.parked.get(&id).map(|view| &view.sidebar),
        }
    }

    /// The selected rows that are on screen: what Copy takes, like a drag does.
    pub fn visible_selection(&self) -> SidebarSelection {
        let visible = self
            .selection
            .session()
            .and_then(|session| {
                let sidebar = self.sidebar_of(session)?;
                Some(visible_nodes(session, sidebar))
            })
            .unwrap_or_default();
        self.selection.clone().visible_only(&visible)
    }

    /// The label key and count of the Copy entry for the current selection (`None`: disabled).
    pub fn copy_entry(&self) -> Option<(&'static str, u64)> {
        copy_entry(&self.visible_selection())
    }

    /// The Paste entry for `target` against the stored clipboard.
    pub fn paste_entry(&self, target: &PasteTarget) -> PasteEntry {
        self.with_paste_context(|ctx| paste_entry(self.clipboard.as_ref(), target, ctx))
    }

    /// What the paste rules ask about the open sessions. `existing_names(session, "")` lists the
    /// databases of the session and `existing_names(session, db)` the collections of `db`, as far
    /// as the sidebar has loaded them: a database whose collections are not loaded yet reports
    /// none, and a paste never fetches anything.
    pub(super) fn with_paste_context<R>(&self, f: impl FnOnce(&PasteContext) -> R) -> R {
        f(&PasteContext {
            // Failed and connecting roots are in `open` too, but have no live client to copy with.
            open: &|session| self.status_of(session) == Some(ConnStatus::Connected),
            read_only: &|session| self.session_read_only(session),
            existing_names: &|session, db| {
                let Some(sidebar) = self.sidebar_of(session) else {
                    return Vec::new();
                };
                match db {
                    "" => sidebar.databases.iter().map(|d| d.name.clone()).collect(),
                    _ => sidebar
                        .databases
                        .iter()
                        .find(|d| d.name == db)
                        .and_then(|d| d.collections.clone())
                        .unwrap_or_default(),
                }
            },
        })
    }
}

/// A plain click makes the clicked node the single selection (on top of what the click does).
pub(super) fn clicked(state: AppState, session: Uuid, action: &SidebarAction) -> AppState {
    let node = match action {
        SidebarAction::Open(ns) => NodeRef::Collection {
            session,
            ns: ns.clone(),
        },
        SidebarAction::Toggle(db) | SidebarAction::LoadCollections(db) => NodeRef::Database {
            session,
            db: db.clone(),
        },
        _ => return state,
    };
    AppState {
        selection: state.selection.single(node),
        ..state
    }
}

/// Applies a modified click or a focus move. Never opens or expands anything.
pub(super) fn select(state: AppState, node: NodeRef, mode: SelectMode) -> Out {
    let session = node.session();
    let Some(sidebar) = state.sidebar_of(session) else {
        return (state, Vec::new());
    };
    let visible = visible_nodes(session, sidebar);
    let selection = state.selection.clone().select(node, mode, &visible);
    (AppState { selection, ..state }, Vec::new())
}

/// Stores the visible selection as the clipboard. A selection that cannot be copied keeps the old
/// clipboard and says so.
pub(super) fn copy(state: AppState) -> Out {
    let next = match clipboard_from_selection(&state.visible_selection()) {
        Some(clipboard) => AppState {
            clipboard: Some(clipboard),
            ..state.without_banner()
        },
        None => AppState {
            banner: Some(Banner::CopyUnavailable),
            banner_session: None,
            ..state
        },
    };
    (next, Vec::new())
}

/// Pastes the clipboard onto `target`: the request waits in `pending_paste` for the paste dialog,
/// or the reason it cannot be pasted is shown. Nothing is sent to the runtime here.
pub(super) fn paste(state: AppState, target: PasteTarget) -> Out {
    // One paste dialog at a time: it is modal, and a copy under way is not disturbed.
    if state.copy.is_some() {
        return (state, Vec::new());
    }
    let outcome = match &state.clipboard {
        Some(clipboard) => state.with_paste_context(|ctx| clipboard.paste_onto(&target, ctx)),
        None => Err(PasteBlock::EmptyClipboard),
    };
    let next = match outcome {
        Ok(request) => AppState {
            pending_paste: Some(request),
            ..state.without_banner()
        },
        Err(block) => AppState {
            pending_paste: None,
            banner: Some(Banner::PasteBlocked(block)),
            banner_session: None,
            ..state
        },
    };
    (next, Vec::new())
}
