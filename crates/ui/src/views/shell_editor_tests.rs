use super::*;
use egui_kittest::Harness;

#[test]
fn ascii_selections_keep_their_offsets() {
    assert_eq!(char_range_to_bytes("db.find()", 3..7), Some(3..7));
    assert_eq!(byte_range_to_chars("db.find()", &(3..7)), 3..7);
}

#[test]
fn multi_byte_text_is_converted_both_ways() {
    let text = "é日😀x";
    // é = 2 bytes, 日 = 3 bytes, 😀 = 4 bytes.
    assert_eq!(char_range_to_bytes(text, 1..3), Some(2..9));
    assert_eq!(char_range_to_bytes(text, 0..4), Some(0..10));
    assert_eq!(byte_range_to_chars(text, &(2..9)), 1..3);
    // A byte inside a character falls back to that character's start.
    assert_eq!(byte_range_to_chars(text, &(3..6)), 1..2);
}

#[test]
fn an_empty_or_reversed_selection_is_none() {
    assert_eq!(char_range_to_bytes("abc", 2..2), None);
    let reversed = Range { start: 3, end: 1 };
    assert_eq!(char_range_to_bytes("abc", reversed), None);
    assert_eq!(char_range_to_bytes("", 0..1), None);
}

#[test]
fn offsets_past_the_end_stay_inside_the_text() {
    assert_eq!(char_range_to_bytes("ab", 1..9), Some(1..2));
    assert_eq!(byte_range_to_chars("ab", &(1..99)), 1..2);
}

struct Frame {
    shell: ShellState,
    edits: Vec<ShellAction>,
}

/// The editor alone; its edits are applied back to the shell like the reducer would.
fn harness(shell: ShellState) -> Harness<'static, Frame> {
    let i18n = I18n::english();
    let mut harness = Harness::new_ui_state(
        move |ui, frame: &mut Frame| {
            let Some(edit) = show_editor(ui, &frame.shell, 200.0, &i18n) else {
                return;
            };
            if let ShellAction::Edit { script, selection } = &edit {
                frame.shell = frame.shell.edited(script.clone(), selection.clone());
            }
            frame.edits.push(edit);
        },
        Frame {
            shell,
            edits: Vec::new(),
        },
    );
    harness.run_steps(2);
    harness
}

fn cursor_chars(h: &Harness<'static, Frame>) -> Option<Range<usize>> {
    egui::text_edit::TextEditState::load(&h.ctx, egui::Id::new(EDITOR_ID))
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.as_sorted_char_range())
}

fn select_chars(h: &Harness<'static, Frame>, chars: Range<usize>) {
    let id = egui::Id::new(EDITOR_ID);
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap_or_default();
    let range = CCursorRange::two(
        egui::text::CCursor::new(chars.start),
        egui::text::CCursor::new(chars.end),
    );
    state.cursor.set_char_range(Some(range));
    state.store(&h.ctx, id);
}

#[test]
fn a_selection_in_multi_byte_text_is_reported_in_bytes() {
    let mut h = harness(ShellState::default().edited("é日x".to_string(), None));
    select_chars(&h, 1..2);
    h.run_steps(2);
    assert_eq!(h.state().shell.selection, Some(2..5));
    assert_eq!(h.state().shell.script, "é日x");
}

#[test]
fn a_collapsed_caret_is_no_selection() {
    let mut h = harness(ShellState::default().edited("abc".to_string(), Some(0..2)));
    assert_eq!(
        cursor_chars(&h),
        Some(0..2),
        "the first frame shows the state's selection"
    );
    select_chars(&h, 1..1);
    h.run_steps(2);
    assert_eq!(h.state().shell.selection, None);
}

#[test]
fn the_states_selection_is_applied_only_when_the_state_changed_the_script() {
    let mut h = harness(ShellState::default().edited("ab".to_string(), None));
    // Like Format: the script and its selection change outside the editor.
    h.state_mut().shell = h.state().shell.edited("日本".to_string(), Some(3..6));
    h.run_steps(2);
    assert_eq!(cursor_chars(&h), Some(1..2));
    // The user moves the selection; the state's old one is not pushed back.
    let edits_before = h.state().edits.len();
    select_chars(&h, 0..1);
    h.run_steps(3);
    assert_eq!(cursor_chars(&h), Some(0..1));
    assert_eq!(h.state().shell.selection, Some(0..3));
    assert_eq!(
        h.state().edits.len(),
        edits_before + 1,
        "one edit, then quiet"
    );
}

#[test]
fn nothing_is_reported_while_nothing_changes() {
    let mut h = harness(ShellState::default().edited("db.a.find()".to_string(), None));
    h.run_steps(5);
    assert!(h.state().edits.is_empty(), "{:?}", h.state().edits);
}
