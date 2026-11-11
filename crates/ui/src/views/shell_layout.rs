//! How the Shell tab shares its height between the editor, the History list and the output pane.
//!
//! With the History panel closed the editor takes its usual share and the output the rest. With
//! it open, the panel's room is reserved BEFORE the editor is sized: the output pane keeps
//! [`OUTPUT_MIN_HEIGHT`] while the window allows it, and the editor and the list shrink first
//! (each down to a floor that keeps it usable).
//!
//! Everything else the tab draws below the toolbar (the hints, a notice, the panel's toggle, note
//! and clear controls, the separator) is its "chrome". Its height depends on wrapping and on the
//! notice shown, so it is measured in one frame and used in the next ([`remember_chrome`]); a
//! change asks for one more frame, so the layout settles at once.
use eframe::egui;
use uuid::Uuid;

/// The share of the tab's height the editor takes while the History panel is closed.
const EDITOR_SHARE: f32 = 0.45;
/// The editor is never shorter than this while the panel is closed (points).
const EDITOR_MIN_HEIGHT: f32 = 120.0;
/// The editor is never shorter than this while the panel is open: about four lines (points).
pub(crate) const EDITOR_MIN_OPEN_HEIGHT: f32 = 64.0;
/// The output pane keeps this much while the window leaves room for it (points).
pub(crate) const OUTPUT_MIN_HEIGHT: f32 = 120.0;
/// The open list takes at most this share of the tab's height.
const LIST_SHARE: f32 = 0.25;
/// ... and never more than this (points), about five rows.
pub(crate) const LIST_MAX_HEIGHT: f32 = 180.0;
/// The open list always shows at least this much (points): one row.
pub(crate) const LIST_MIN_HEIGHT: f32 = 32.0;
/// The chrome assumed before a frame has measured it (points).
const CHROME_GUESS: f32 = 220.0;
/// Room kept for the item spacing around the editor and the list, which the measurement leaves
/// out (points).
const SPACING_SLACK: f32 = 16.0;

/// The heights the editor and the History list get in one frame (points). `list` is the most the
/// list may take; it is 0 while the panel is closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Heights {
    pub editor: f32,
    pub list: f32,
}

/// The heights for a tab with `available` points below its toolbar. `chrome` is what the rest
/// took in the last frame; it only matters while the panel is `open`.
pub(crate) fn heights(available: f32, open: bool, chrome: f32) -> Heights {
    let editor = (available * EDITOR_SHARE).max(EDITOR_MIN_HEIGHT);
    if !open {
        return Heights { editor, list: 0.0 };
    }
    let list = LIST_MAX_HEIGHT.min(available * LIST_SHARE);
    let room = (available - chrome - SPACING_SLACK - OUTPUT_MIN_HEIGHT).max(0.0);
    if editor + list <= room {
        return Heights { editor, list };
    }
    // Both shrink in proportion, each to its floor (one at its floor leaves the rest to the
    // other); past the floors the output gives way.
    let scaled = (list * room / (editor + list)).max(LIST_MIN_HEIGHT);
    let editor = (room - scaled).max(EDITOR_MIN_OPEN_HEIGHT);
    let list = (room - editor).clamp(LIST_MIN_HEIGHT, scaled);
    Heights { editor, list }
}

fn chrome_id(session: Uuid) -> egui::Id {
    egui::Id::new(("shell_layout_chrome", session))
}

/// The chrome `session`'s tab measured last, or a guess before the first measurement.
pub(crate) fn last_chrome(ui: &egui::Ui, session: Uuid) -> f32 {
    ui.data(|d| d.get_temp::<f32>(chrome_id(session)))
        .unwrap_or(CHROME_GUESS)
}

/// Keeps this frame's chrome for the next; a change of more than a point asks for another frame
/// so the heights follow it without waiting for input.
pub(crate) fn remember_chrome(ui: &egui::Ui, session: Uuid, chrome: f32) {
    let id = chrome_id(session);
    let previous = ui.data(|d| d.get_temp::<f32>(id));
    ui.data_mut(|d| d.insert_temp(id, chrome));
    let settled = previous.is_some_and(|p| (p - chrome).abs() <= 1.0);
    if !settled {
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
#[path = "shell_layout_tests.rs"]
mod tests;
