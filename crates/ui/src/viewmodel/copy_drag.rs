//! Dragging sidebar rows onto a target, as pure decisions: what a drag carries and whether a
//! target takes it. Both reuse the Copy and Paste rules, so a drag can never do what the menu
//! would refuse.
use super::copy_clipboard::{CopyClipboard, PasteContext, PasteTarget};
use super::copy_menu::paste_block_key;
use super::sidebar_selection::{NodeRef, SidebarSelection, clipboard_from_selection};
use std::collections::HashMap;
use std::sync::Arc;

/// Whether a target takes what is being dragged over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropVerdict {
    Accept,
    /// Refused; the i18n key says why.
    Blocked(&'static str),
}

/// What dragging `dragged` takes along: the whole selection when the row is part of it (only the
/// rows that are on screen), else the row alone.
pub fn drag_selection(
    selection: &SidebarSelection,
    dragged: &NodeRef,
    visible: &[NodeRef],
) -> SidebarSelection {
    if selection.is_selected(dragged) {
        selection.clone().visible_only(visible)
    } else {
        SidebarSelection::default().single(dragged.clone())
    }
}

/// The clipboard a drag of `dragged` carries (the same rule as the Copy menu entry), or `None`
/// when what it would take cannot be copied.
pub fn drag_payload(
    selection: &SidebarSelection,
    dragged: &NodeRef,
    visible: &[NodeRef],
) -> Option<CopyClipboard> {
    clipboard_from_selection(&drag_selection(selection, dragged, visible))
}

/// Whether `payload` may be dropped on `target`: the paste rules decide.
pub fn drop_verdict(
    payload: &CopyClipboard,
    target: &PasteTarget,
    ctx: &PasteContext,
) -> DropVerdict {
    match payload.paste_onto(target, ctx) {
        Ok(_) => DropVerdict::Accept,
        Err(block) => DropVerdict::Blocked(paste_block_key(block)),
    }
}

/// What a verdict depends on, cheaply readable (no name list is cloned): whether the source and
/// the target are open, whether the target is read-only, and how many databases and collections
/// the target side has loaded, plus the generation of its name lists (a renamed collection keeps
/// every count). A drag lasts seconds, so a refresh shows as a new generation; a connection that
/// opens, fails or is locked changes the booleans.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DropContextKey {
    pub source_open: bool,
    pub target_open: bool,
    pub target_read_only: bool,
    pub databases: usize,
    /// The collections loaded for the target database; `None` when not loaded (or no database).
    pub collections: Option<usize>,
    /// [`SidebarState::generation`](super::sidebar::SidebarState) of the target's sidebar.
    pub names_generation: u64,
}

/// The verdicts of one drag, per target, so a frame does not run the paste rules again for a
/// row whose context has not changed. It belongs to the payload of that drag and is dropped
/// when the drag ends. Memoization only: a lookup never changes what a verdict is.
#[derive(Clone, Debug)]
pub struct DropVerdictCache {
    payload: Arc<CopyClipboard>,
    entries: HashMap<PasteTarget, (DropContextKey, DropVerdict)>,
}

impl DropVerdictCache {
    pub fn new(payload: Arc<CopyClipboard>) -> Self {
        Self {
            payload,
            entries: HashMap::new(),
        }
    }

    /// The cache is for this very drag payload.
    pub fn is_for(&self, payload: &Arc<CopyClipboard>) -> bool {
        Arc::ptr_eq(&self.payload, payload)
    }

    /// The verdict for `target` under `key`: the remembered one, else `compute()` (remembered
    /// for the next frame, replacing an entry of an older context).
    pub fn verdict(
        &mut self,
        target: &PasteTarget,
        key: DropContextKey,
        compute: impl FnOnce() -> DropVerdict,
    ) -> DropVerdict {
        if let Some((remembered, verdict)) = self.entries.get(target) {
            if *remembered == key {
                return *verdict;
            }
        }
        let verdict = compute();
        self.entries.insert(target.clone(), (key, verdict));
        verdict
    }
}
