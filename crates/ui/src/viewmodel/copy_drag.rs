//! Dragging sidebar rows onto a target, as pure decisions: what a drag carries and whether a
//! target takes it. Both reuse the Copy and Paste rules, so a drag can never do what the menu
//! would refuse.
use super::copy_clipboard::{CopyClipboard, PasteContext, PasteTarget};
use super::copy_menu::paste_block_key;
use super::sidebar_selection::{NodeRef, SidebarSelection, clipboard_from_selection};

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
