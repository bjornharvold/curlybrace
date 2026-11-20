//! The keyboard shortcuts of the split view (#159): Cmd+\ splits the view or ends the split, F6
//! and Shift+F6 move focus to the other pane (with two panes, forward and back are the same
//! move). Each chord matches exactly, as the tab shortcuts do. The caller decides when they are
//! live: F6 is not a text key, so it works with a text field focused, where Cmd+\ is held back
//! like every other tab shortcut; both wait while a dialog is open.
use super::tab_shortcuts::consume_exact;
use crate::app::tab_actions::TabAction;
use eframe::egui::{Context, Key, Modifiers};

const CMD: Modifiers = Modifiers::COMMAND;

/// What the keys pressed this frame ask for of the split. `chords` is whether Cmd+\ is live,
/// `pane_keys` whether F6 is. Each key is consumed.
pub fn split_shortcuts(ctx: &Context, chords: bool, pane_keys: bool) -> Vec<TabAction> {
    ctx.input_mut(|input| {
        let mut wanted = Vec::new();
        // The physical backslash too: on a layout that types it with another key, the logical key
        // is not the one the chord names.
        if chords && consume_exact(input, CMD, Key::Backslash, true) {
            wanted.push(TabAction::ToggleSplit);
        }
        if pane_keys {
            let forward = consume_exact(input, Modifiers::NONE, Key::F6, false);
            let back = consume_exact(input, Modifiers::SHIFT, Key::F6, false);
            if forward || back {
                wanted.push(TabAction::FocusOtherPane);
            }
        }
        wanted
    })
}
