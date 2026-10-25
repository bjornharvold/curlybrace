//! Selecting sidebar rows, Copy and Paste. Pure like the rest of the reducer.
use super::reducer::Out;
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{
    NodeRef, PasteContext, PasteEntry, PasteTarget, SelectMode, SidebarAction, SidebarState,
    clipboard_from_selection, copy_entry, paste_entry, visible_nodes,
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

    /// The label key and count of the Copy entry for the current selection (`None`: disabled).
    pub fn copy_entry(&self) -> Option<(&'static str, u64)> {
        copy_entry(&self.selection)
    }

    /// The Paste entry for `target` against the stored clipboard.
    pub fn paste_entry(&self, target: &PasteTarget) -> PasteEntry {
        self.with_paste_context(|ctx| paste_entry(self.clipboard.as_ref(), target, ctx))
    }

    /// What the paste rules ask about the open sessions. `existing_names(session, "")` lists the
    /// databases of the session and `existing_names(session, db)` the collections of `db`, as far
    /// as the sidebar has loaded them: a database whose collections are not loaded yet reports
    /// none, and a paste never fetches anything.
    fn with_paste_context<R>(&self, f: impl FnOnce(&PasteContext) -> R) -> R {
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

/// Stores the selection as the clipboard; a selection that cannot be copied changes nothing.
pub(super) fn copy(state: AppState) -> Out {
    let clipboard = clipboard_from_selection(&state.selection).or(state.clipboard);
    (AppState { clipboard, ..state }, Vec::new())
}

/// Pastes the clipboard onto `target`: the request waits in `pending_paste` for the paste dialog,
/// or the reason it cannot be pasted is shown. Nothing is sent to the runtime here.
pub(super) fn paste(state: AppState, target: PasteTarget) -> Out {
    // One paste dialog at a time: it is modal, and a copy under way is not disturbed.
    if state.copy.is_some() {
        return (state, Vec::new());
    }
    let Some(clipboard) = &state.clipboard else {
        return (state, Vec::new());
    };
    let outcome = state.with_paste_context(|ctx| clipboard.paste_onto(&target, ctx));
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
