use super::*;

fn mods(command: bool, shift: bool) -> egui::Modifiers {
    egui::Modifiers {
        command,
        shift,
        ..Default::default()
    }
}

#[test]
fn a_modified_click_changes_the_selection_and_a_plain_click_does_not() {
    assert_eq!(click_mode(&mods(false, false)), None);
    assert_eq!(click_mode(&mods(true, false)), Some(SelectMode::Toggle));
    assert_eq!(click_mode(&mods(false, true)), Some(SelectMode::Extend));
    assert_eq!(click_mode(&mods(true, true)), Some(SelectMode::Toggle));
}

#[test]
fn keyboard_focus_selects_the_row_extends_with_shift_and_keeps_with_command() {
    assert_eq!(focus_mode(&mods(false, false)), Some(SelectMode::Single));
    assert_eq!(focus_mode(&mods(false, true)), Some(SelectMode::Extend));
    assert_eq!(focus_mode(&mods(true, false)), None);
}
