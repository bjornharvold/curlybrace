//! The keyboard shortcuts of the tabs (#159). `Cmd` is `Modifiers::COMMAND` (Ctrl off macOS).
//! None of these keys types text, so they work while a text field has the keyboard too (egui's
//! `wants_keyboard_input` is true for any focused widget, a button included, so it cannot tell
//! typing from a focused tab); they are held back while a dialog is open (`enabled`). None is
//! bound by eframe, egui or winit's macOS menu (which binds only Cmd+Q, Cmd+H and Cmd+Alt+H);
//! egui's own keys are Tab / Shift+Tab and the arrows for focus, and Cmd+Plus / Minus / 0 for
//! zoom. Ctrl+Tab therefore also moves keyboard focus one widget, which egui does for every Tab
//! press before the app sees it.
use crate::app::tab_actions::TabAction;
use eframe::egui::{Context, Key, Modifiers};

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
            .filter(|(mods, key, _)| input.consume_key(*mods, *key))
            .map(|(_, _, action)| action)
            .collect()
    })
}
