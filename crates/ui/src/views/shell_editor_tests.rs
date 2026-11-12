use super::*;
use egui_kittest::Harness;
use std::collections::BTreeMap;

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

const A: Uuid = Uuid::from_u128(0xA);
const B: Uuid = Uuid::from_u128(0xB);

struct Frame {
    /// The session whose editor is drawn.
    session: Uuid,
    shells: BTreeMap<Uuid, ShellState>,
    edits: Vec<(Uuid, ShellAction)>,
}

impl Frame {
    fn shell(&self, session: Uuid) -> &ShellState {
        &self.shells[&session]
    }
}

fn script(text: &str, selection: Option<Range<usize>>) -> ShellState {
    ShellState::default().edited(text.to_string(), selection)
}

/// The editor of the drawn session; its edits are applied back to that session's shell like
/// the reducer would.
fn harness_of(shells: BTreeMap<Uuid, ShellState>, session: Uuid) -> Harness<'static, Frame> {
    let i18n = I18n::english();
    let mut harness = Harness::new_ui_state(
        move |ui, frame: &mut Frame| {
            let session = frame.session;
            let Some(edit) = show_editor(ui, frame.shell(session), session, 200.0, &[], &i18n)
            else {
                return;
            };
            if let ShellAction::Edit { script, selection } = &edit {
                let next = frame
                    .shell(session)
                    .edited(script.clone(), selection.clone());
                frame.shells.insert(session, next);
            }
            frame.edits.push((session, edit));
        },
        Frame {
            session,
            shells,
            edits: Vec::new(),
        },
    );
    harness.run_steps(2);
    harness
}

fn harness(shell: ShellState) -> Harness<'static, Frame> {
    harness_of(BTreeMap::from([(A, shell)]), A)
}

fn cursor_chars(h: &Harness<'static, Frame>, session: Uuid) -> Option<Range<usize>> {
    egui::text_edit::TextEditState::load(&h.ctx, editor_id(session))
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.as_sorted_char_range())
}

fn select_chars(h: &Harness<'static, Frame>, session: Uuid, chars: Range<usize>) {
    let id = editor_id(session);
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap_or_default();
    let range = CCursorRange::two(
        egui::text::CCursor::new(chars.start),
        egui::text::CCursor::new(chars.end),
    );
    state.cursor.set_char_range(Some(range));
    state.store(&h.ctx, id);
}

fn focus_editor(h: &mut Harness<'static, Frame>, session: Uuid) {
    h.ctx.memory_mut(|m| m.request_focus(editor_id(session)));
    h.run_steps(2);
}

#[test]
fn a_selection_in_multi_byte_text_is_reported_in_bytes() {
    let mut h = harness(script("é日x", None));
    select_chars(&h, A, 1..2);
    h.run_steps(2);
    assert_eq!(h.state().shell(A).selection, Some(2..5));
    assert_eq!(h.state().shell(A).script, "é日x");
}

#[test]
fn a_collapsed_caret_is_no_selection() {
    let mut h = harness(script("abc", Some(0..2)));
    assert_eq!(
        cursor_chars(&h, A),
        Some(0..2),
        "the first frame shows the state's selection"
    );
    select_chars(&h, A, 1..1);
    h.run_steps(2);
    assert_eq!(h.state().shell(A).selection, None);
}

#[test]
fn the_states_selection_is_applied_only_when_the_state_changed_the_script() {
    let mut h = harness(script("ab", None));
    // Like Format: the script and its selection change outside the editor.
    h.state_mut().shells.insert(A, script("日本", Some(3..6)));
    h.run_steps(2);
    assert_eq!(cursor_chars(&h, A), Some(1..2));
    // The user moves the selection; the state's old one is not pushed back.
    let edits_before = h.state().edits.len();
    select_chars(&h, A, 0..1);
    h.run_steps(3);
    assert_eq!(cursor_chars(&h, A), Some(0..1));
    assert_eq!(h.state().shell(A).selection, Some(0..3));
    assert_eq!(
        h.state().edits.len(),
        edits_before + 1,
        "one edit, then quiet"
    );
}

#[test]
fn a_selection_the_state_moved_on_its_own_reaches_the_text_box_without_taking_the_focus() {
    let mut h = harness(script("db.a.find(\nx", None));
    let edits_before = h.state().edits.len();
    // Like a Format that left the text: only the selection changes, outside the editor.
    let moved = h
        .state()
        .shell(A)
        .edited("db.a.find(\nx".into(), Some(9..10));
    h.state_mut().shells.insert(A, moved);
    h.run_steps(2);
    assert_eq!(cursor_chars(&h, A), Some(9..10));
    assert_eq!(h.state().shell(A).selection, Some(9..10));
    assert_eq!(h.state().edits.len(), edits_before, "nothing reported back");
    assert!(!h.ctx.memory(|m| m.has_focus(editor_id(A))));
}

/// The shell after a Run stopped on a syntax problem, then Go to problem.
fn gone_to_problem(shell: &ShellState) -> ShellState {
    shell.problem_selected()
}

#[test]
fn every_go_to_problem_focuses_the_editor_and_shows_the_problem() {
    let broken = script("db.a.find(\nx", None).run_requested().0;
    let mut h = harness(broken);
    assert!(!h.ctx.memory(|m| m.has_focus(editor_id(A))));
    let first = gone_to_problem(h.state().shell(A));
    h.state_mut().shells.insert(A, first);
    h.run_steps(2);
    assert!(h.ctx.memory(|m| m.has_focus(editor_id(A))));
    assert_eq!(cursor_chars(&h, A), Some(9..9));
    // The focus moves away (the button was clicked), the editor is not touched, and Go to
    // problem is clicked again: the same selection, focused again.
    h.ctx.memory_mut(|m| m.surrender_focus(editor_id(A)));
    h.run_steps(2);
    assert!(!h.ctx.memory(|m| m.has_focus(editor_id(A))));
    let second = gone_to_problem(h.state().shell(A));
    assert_eq!(second.selection, h.state().shell(A).selection);
    h.state_mut().shells.insert(A, second);
    h.run_steps(2);
    assert!(h.ctx.memory(|m| m.has_focus(editor_id(A))));
    assert_eq!(cursor_chars(&h, A), Some(9..9));
}

#[test]
fn a_shell_state_created_anew_for_the_same_session_does_not_take_the_focus() {
    let broken = script("db.a.find(\nx", None).run_requested().0;
    let mut h = harness(broken);
    let twice = gone_to_problem(&gone_to_problem(h.state().shell(A)));
    h.state_mut().shells.insert(A, twice);
    h.run_steps(2);
    h.ctx.memory_mut(|m| m.surrender_focus(editor_id(A)));
    h.run_steps(2);
    // The session's shell is replaced by a fresh one (its focus counter starts over).
    let fresh = script("db.a.find(\nx", None).run_requested().0;
    h.state_mut().shells.insert(A, fresh);
    h.run_steps(2);
    assert!(!h.ctx.memory(|m| m.has_focus(editor_id(A))));
    // Its own first Go to problem still focuses.
    let asked = gone_to_problem(h.state().shell(A));
    h.state_mut().shells.insert(A, asked);
    h.run_steps(2);
    assert!(h.ctx.memory(|m| m.has_focus(editor_id(A))));
}

#[test]
fn the_first_frame_does_not_take_the_focus() {
    let h = harness(script("abc", Some(0..2)));
    assert!(!h.ctx.memory(|m| m.has_focus(editor_id(A))));
}

#[test]
fn an_outside_change_without_a_selection_collapses_the_old_one() {
    let mut h = harness(script("abcd", Some(0..3)));
    assert_eq!(cursor_chars(&h, A), Some(0..3));
    h.state_mut().shells.insert(A, script("xy", None));
    h.run_steps(2);
    let caret = cursor_chars(&h, A).expect("a caret");
    assert!(caret.is_empty(), "{caret:?}");
    assert_eq!(h.state().shell(A).selection, None);
}

#[test]
fn nothing_is_reported_while_nothing_changes() {
    let mut h = harness(script("db.a.find()", None));
    h.run_steps(5);
    assert!(h.state().edits.is_empty(), "{:?}", h.state().edits);
}

#[test]
fn each_session_keeps_its_own_selection() {
    let shells = BTreeMap::from([(A, script("aaaa", Some(0..3))), (B, script("bbbb", None))]);
    let mut h = harness_of(shells, A);
    assert_eq!(cursor_chars(&h, A), Some(0..3));
    h.state_mut().session = B;
    h.run_steps(2);
    assert_eq!(
        h.state().shell(B).selection,
        None,
        "A's selection is not B's"
    );
    assert!(
        h.state().edits.iter().all(|(s, _)| *s != B),
        "{:?}",
        h.state().edits
    );
    // Typing in B reports B's text and no selection.
    focus_editor(&mut h, B);
    h.input_mut().events.push(egui::Event::Text("!".into()));
    h.run_steps(2);
    let last = h.state().edits.last().cloned();
    let expected = ShellAction::Edit {
        script: "bbbb!".into(),
        selection: None,
    };
    assert_eq!(last, Some((B, expected)));
    // Back in A, its selection is where it was.
    h.state_mut().session = A;
    h.run_steps(2);
    assert_eq!(cursor_chars(&h, A), Some(0..3));
    assert_eq!(h.state().shell(A).script, "aaaa");
}

#[test]
fn undo_in_one_session_never_brings_back_another_sessions_text() {
    let shells = BTreeMap::from([(A, script("aaa", None)), (B, script("bbb", None))]);
    let mut h = harness_of(shells, A);
    focus_editor(&mut h, A);
    h.input_mut().events.push(egui::Event::Text("1".into()));
    h.run_steps(2);
    assert_eq!(h.state().shell(A).script, "aaa1");
    h.state_mut().session = B;
    h.run_steps(2);
    focus_editor(&mut h, B);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().shell(B).script, "bbb");
    assert_eq!(h.state().shell(A).script, "aaa1");
}

#[test]
fn typing_after_go_to_problem_inserts_at_the_caret_and_keeps_the_bracket() {
    let broken = script("db.a.find(\nx", None).run_requested().0;
    let mut h = harness(broken);
    let gone = gone_to_problem(h.state().shell(A));
    h.state_mut().shells.insert(A, gone);
    h.run_steps(2);
    h.input_mut().events.push(egui::Event::Text("y".into()));
    h.run_steps(2);
    assert_eq!(h.state().shell(A).script, "db.a.findy(\nx");
    // The caret alone is no selection: nothing about it is reported as one.
    assert_eq!(h.state().shell(A).selection, None);
}
