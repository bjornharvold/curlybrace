//! The keyboard shortcuts of the tabs (#159). `Cmd` is `Modifiers::COMMAND` (Ctrl off macOS).
//! They are consumed only while no text field has the keyboard (typing is never taken) and no
//! dialog is open: the caller decides (`enabled`, see `app::tab_strip_panel`, which tells a text
//! field from a focused button with `text_has_focus`; egui's `wants_keyboard_input` is true for
//! both). Each chord matches exactly: Cmd+Shift+W is not Cmd+W. None is
//! bound by eframe, egui or winit's macOS menu (which binds only Cmd+Q, Cmd+H and Cmd+Alt+H);
//! egui's own keys are Tab / Shift+Tab and the arrows for focus, and Cmd+Plus / Minus / 0 for
//! zoom. Ctrl+Tab therefore also moves keyboard focus one widget, which egui does for every Tab
//! press before the app sees it.
use crate::app::tab_actions::TabAction;
use eframe::egui::{Context, Event, InputState, Key, Modifiers};

const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers {
    shift: true,
    ..Modifiers::COMMAND
};
const CTRL: Modifiers = Modifiers::CTRL;
const CTRL_SHIFT: Modifiers = Modifiers {
    shift: true,
    ..Modifiers::CTRL
};

/// The digits Cmd+1 .. Cmd+8 pick a position; Cmd+9 picks the last tab.
const DIGITS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

/// Whether the widget with the keyboard is a text field, scrolled into view or not: egui keeps a
/// `TextEditState` for every text edit it has drawn, and no other widget has one. (The IME
/// rectangle egui reports for a focused field is only reported while the field is visible.)
pub fn text_has_focus(ctx: &Context) -> bool {
    ctx.memory(|m| m.focused())
        .is_some_and(|id| eframe::egui::TextEdit::load_state(ctx, id).is_some())
}

/// What the keys pressed this frame ask for (usually nothing). Each key is consumed.
pub fn tab_shortcuts(ctx: &Context, enabled: bool) -> Vec<TabAction> {
    if !enabled {
        return Vec::new();
    }
    let mut table: Vec<(Modifiers, Key, TabAction)> = vec![
        (CMD_SHIFT, Key::T, TabAction::ReopenClosed),
        (CMD_SHIFT, Key::OpenBracket, TabAction::Previous),
        (CMD_SHIFT, Key::OpenCurlyBracket, TabAction::Previous),
        (CMD_SHIFT, Key::CloseBracket, TabAction::Next),
        (CMD_SHIFT, Key::CloseCurlyBracket, TabAction::Next),
        (CTRL_SHIFT, Key::Tab, TabAction::PreviousMru),
        (CTRL, Key::Tab, TabAction::NextMru),
        (CTRL_SHIFT, Key::PageDown, TabAction::MoveActive(1)),
        (CTRL_SHIFT, Key::PageUp, TabAction::MoveActive(-1)),
        (CMD, Key::T, TabAction::New),
        (CMD, Key::W, TabAction::CloseActive),
        (CMD_SHIFT, Key::D, TabAction::DuplicateActive),
    ];
    for (at, key) in DIGITS.into_iter().enumerate() {
        let action = match at {
            8 => TabAction::Last,
            n => TabAction::Nth(n),
        };
        table.push((CMD, key, action));
    }
    ctx.input_mut(|input| {
        table
            .into_iter()
            .filter(|(mods, key, _)| consume_exact(input, *mods, *key, by_position(*key)))
            .map(|(_, _, action)| action)
            .collect()
    })
}

/// The digits and the brackets are also found by the physical key. egui-winit reports the key a
/// press types (its logical key) first, and on a layout where a digit is typed with Shift (French
/// AZERTY types `'` on the 4 key) or a bracket needs another key combination (German), the logical
/// key is not the one the chord names; the physical key is. The modifiers must still be exactly
/// the chord's, so Cmd+Shift+3 on a layout that types 3 plain is not Cmd+3. Letters stay logical:
/// W and T are where a layout puts them.
fn by_position(key: Key) -> bool {
    DIGITS.contains(&key) || matches!(key, Key::OpenBracket | Key::CloseBracket)
}

/// Consumes `key` pressed with exactly `mods` (and, with `by_position`, the physical `key` with
/// exactly `mods` too). egui's own `consume_key` matches logically: a chord with an extra Shift
/// or Alt still counts, so Cmd+Shift+W would close a tab and Cmd+Shift+3 would pick one. A tab
/// shortcut is the chord and nothing more.
fn consume_exact(input: &mut InputState, mods: Modifiers, key: Key, by_position: bool) -> bool {
    let mut hit = false;
    input.events.retain(|event| {
        let is_it = matches!(
            event,
            Event::Key { key: logical, physical_key, pressed: true, modifiers, .. }
                if modifiers.matches_exact(mods)
                    && (*logical == key || (by_position && *physical_key == Some(key)))
        );
        hit |= is_it;
        !is_it
    });
    hit
}
