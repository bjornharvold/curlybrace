use super::*;
use crate::viewmodel::shell::ShellState;
use crate::views::shell_editor::{editor_id, show_editor};
use eframe::egui::accesskit::Role;
use eframe::egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0xA);

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

// --- applying a pick ----------------------------------------------------------------------

#[test]
fn a_pick_replaces_the_prefix_and_puts_the_caret_after_it() {
    assert_eq!(applied("db.us", &(3..5), "users"), ("db.users".into(), 8));
    assert_eq!(applied("pri", &(0..3), "print"), ("print".into(), 5));
}

#[test]
fn a_pick_in_the_middle_of_a_word_replaces_the_rest_of_it() {
    // db.us|ers.find(): the engine replaces `us`; the pick also takes `ers`.
    let (text, caret) = applied("db.usxs.find()", &(3..5), "users");
    assert_eq!(text, "db.users.find()");
    assert_eq!(caret, 8);
    let (text, _) = applied("{ $ma$x: 1 }", &(2..5), "$match");
    assert_eq!(text, "{ $match: 1 }");
}

#[test]
fn a_pick_keeps_what_follows_a_non_word_byte() {
    let (text, caret) = applied("db.my coll", &(0..5), "db.getCollection(\"my coll\")");
    assert_eq!(text, "db.getCollection(\"my coll\") coll");
    assert_eq!(caret, 27);
    let (text, caret) = applied("é db.u", &(6..7), "users");
    assert_eq!(text, "é db.users");
    assert_eq!(caret, text.len());
}

// --- when the list opens, follows and closes (pure) ---------------------------------------

fn moment(text: &str, caret: usize) -> Moment<'_> {
    Moment {
        text,
        caret: Some(caret),
        focused: true,
        typed: false,
        forced: false,
        changed: false,
    }
}

fn typed(text: &str) -> Moment<'_> {
    Moment {
        typed: true,
        changed: true,
        ..moment(text, text.len())
    }
}

fn labels(popup: &Option<Popup>) -> Vec<&str> {
    popup
        .as_ref()
        .map(|p| {
            p.completion
                .items
                .iter()
                .map(|s| s.label.as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn typing_a_word_character_a_dot_or_a_dollar_opens_the_list() {
    let users = names(&["users"]);
    let open = next_popup(None, &typed("db.us"), &users);
    assert_eq!(labels(&open), ["users"]);
    assert_eq!(open.unwrap().selected, 0);
    assert!(next_popup(None, &typed("db."), &users).is_some());
    assert!(next_popup(None, &typed("f({ $"), &users).is_some());
}

#[test]
fn moving_the_caret_or_editing_without_typing_does_not_open_it() {
    let users = names(&["users"]);
    let moved = Moment {
        changed: true,
        ..moment("db.us", 5)
    };
    assert_eq!(next_popup(None, &moved, &users), None);
}

#[test]
fn the_list_follows_the_word_and_closes_when_the_caret_leaves_it() {
    let users = names(&["users", "usage"]);
    let open = next_popup(None, &typed("db.us"), &users);
    let narrowed = next_popup(open.clone(), &typed("db.use"), &users);
    assert_eq!(labels(&narrowed), ["users"]);
    let back = Moment {
        changed: true,
        ..moment("db.us", 4)
    };
    assert!(
        next_popup(open.clone(), &back, &users).is_some(),
        "still in the word"
    );
    let left = Moment {
        changed: true,
        ..moment("db.us", 2)
    };
    assert_eq!(next_popup(open.clone(), &left, &users), None);
    let space = Moment {
        typed: false,
        changed: true,
        ..moment("db.us ", 6)
    };
    assert_eq!(next_popup(open.clone(), &space, &users), None);
    let nothing = typed("db.usx");
    assert_eq!(next_popup(open, &nothing, &users), None, "no items");
}

#[test]
fn the_list_closes_without_focus_or_with_a_selection() {
    let users = names(&["users"]);
    let open = next_popup(None, &typed("db.us"), &users);
    let unfocused = Moment {
        focused: false,
        ..moment("db.us", 5)
    };
    assert_eq!(next_popup(open.clone(), &unfocused, &users), None);
    let selecting = Moment {
        caret: None,
        ..moment("db.us", 5)
    };
    assert_eq!(next_popup(open, &selecting, &users), None);
}

#[test]
fn on_demand_opens_on_an_empty_word() {
    let forced = Moment {
        forced: true,
        ..moment("", 0)
    };
    assert!(labels(&next_popup(None, &forced, &[])).contains(&"db"));
}

#[test]
fn a_narrower_list_keeps_the_selected_name_when_it_is_still_there() {
    let colls = names(&["usage", "users"]);
    let open = next_popup(None, &typed("db.us"), &colls).map(|p| Popup { selected: 1, ..p });
    let kept = next_popup(open, &typed("db.use"), &colls).unwrap();
    assert_eq!(kept.completion.items[kept.selected].label, "users");
}

#[test]
fn a_word_that_already_is_the_selected_name_closes_the_list() {
    let colls = names(&["users", "user_prefs"]);
    assert_eq!(next_popup(None, &typed("db.users"), &colls), None);
    assert_eq!(next_popup(None, &typed("show dbs"), &[]), None);
    assert_eq!(next_popup(None, &typed("it"), &[]), None);
    let open = next_popup(None, &typed("db.us"), &colls);
    assert_eq!(next_popup(open, &typed("db.users"), &colls), None);
    // Mid-word: `db.us|ers` is the whole word `users`.
    let mid = Moment {
        typed: true,
        changed: true,
        ..moment("db.users", 5)
    };
    assert_eq!(next_popup(None, &mid, &colls), None);
    // No exact match: the list stays.
    assert!(next_popup(None, &typed("db.us"), &colls).is_some());
    // A rewrite inserts other text than was typed, so it stays open.
    let odd = names(&["my-coll"]);
    assert!(next_popup(None, &typed("db.my"), &odd).is_some());
}

// --- the editor with its list (egui_kittest) ----------------------------------------------

struct Frame {
    shell: ShellState,
    collections: Vec<String>,
    edits: usize,
}

fn harness_sized(
    shell: ShellState,
    collections: &[&str],
    size: egui::Vec2,
    top: f32,
) -> Harness<'static, Frame> {
    let i18n = I18n::english();
    let mut h = Harness::builder().with_size(size).build_ui_state(
        move |ui, frame: &mut Frame| {
            ui.add_space(top);
            let edit = show_editor(ui, &frame.shell, A, 120.0, &frame.collections, &i18n);
            if let Some(crate::app::shell_actions::ShellAction::Edit { script, selection }) = edit {
                frame.shell = frame.shell.edited(script, selection);
                frame.edits += 1;
            }
        },
        Frame {
            shell: ShellState::default(),
            collections: names(collections),
            edits: 0,
        },
    );
    h.state_mut().shell = shell;
    h.run_steps(2);
    h
}

fn harness(text: &str, collections: &[&str]) -> Harness<'static, Frame> {
    let shell = ShellState::default().edited(text.to_string(), None);
    harness_sized(shell, collections, egui::vec2(600.0, 400.0), 0.0)
}

/// Focuses the editor with the caret at char `at`.
fn caret_at(h: &mut Harness<'static, Frame>, at: usize) {
    let id = editor_id(A);
    h.ctx.memory_mut(|m| m.request_focus(id));
    let mut state = egui::text_edit::TextEditState::load(&h.ctx, id).unwrap_or_default();
    let caret = egui::text::CCursorRange::one(egui::text::CCursor::new(at));
    state.cursor.set_char_range(Some(caret));
    state.store(&h.ctx, id);
    h.run_steps(2);
}

/// Types `text` one key at a time, like a person (a line break is the Enter key).
fn type_text(h: &mut Harness<'static, Frame>, text: &str) {
    for c in text.chars() {
        match c {
            '\n' => h.key_press(Key::Enter),
            c => h.input_mut().events.push(Event::Text(c.to_string())),
        }
        h.run_steps(2);
    }
}

fn press_with(h: &mut Harness<'static, Frame>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(2);
}

fn press(h: &mut Harness<'static, Frame>, key: Key) {
    h.key_press(key);
    h.run_steps(2);
}

fn caret(h: &Harness<'static, Frame>) -> Option<std::ops::Range<usize>> {
    egui::text_edit::TextEditState::load(&h.ctx, editor_id(A))
        .and_then(|s| s.cursor.char_range())
        .map(|r| r.as_sorted_char_range())
}

fn list_open(h: &Harness<'static, Frame>) -> bool {
    h.query_by_role_and_label(Role::List, &t("shell.complete.list_label"))
        .is_some()
}

fn selected_item(h: &Harness<'static, Frame>) -> Option<String> {
    h.query_all_by_role(Role::ListItem)
        .find(|n| n.accesskit_node().is_selected() == Some(true))
        .and_then(|n| n.accesskit_node().label())
}

/// An empty editor, focused, with `text` typed into it.
fn typed_in(text: &str, collections: &[&str]) -> Harness<'static, Frame> {
    let mut h = harness("", collections);
    caret_at(&mut h, 0);
    type_text(&mut h, text);
    h
}

#[test]
fn typing_db_us_shows_the_list_with_users() {
    let h = typed_in("db.us", &["orders", "users"]);
    assert!(list_open(&h));
    h.get_by_role_and_label(Role::ListItem, "users");
    assert!(
        h.query_by_role_and_label(Role::ListItem, "orders")
            .is_none()
    );
}

#[test]
fn down_and_enter_insert_the_pick_close_the_list_and_keep_the_caret_after_it() {
    let mut h = typed_in("db.us", &["usage", "users"]);
    assert_eq!(
        selected_item(&h).as_deref(),
        Some("usage"),
        "the first is selected"
    );
    press(&mut h, Key::ArrowDown);
    assert_eq!(selected_item(&h).as_deref(), Some("users"));
    press(&mut h, Key::Enter);
    assert_eq!(h.state().shell.script, "db.users");
    assert!(!list_open(&h));
    assert_eq!(caret(&h), Some(8..8));
    assert!(h.ctx.memory(|m| m.has_focus(editor_id(A))));
    // Typing goes on after the insertion.
    type_text(&mut h, ".");
    assert_eq!(h.state().shell.script, "db.users.");
}

#[test]
fn up_wraps_to_the_last_item_and_tab_picks() {
    let mut h = typed_in("db.us", &["usage", "users"]);
    press(&mut h, Key::ArrowUp);
    assert_eq!(selected_item(&h).as_deref(), Some("users"));
    press(&mut h, Key::Tab);
    assert_eq!(h.state().shell.script, "db.users");
}

#[test]
fn esc_closes_the_list_and_leaves_the_text_and_the_focus() {
    let mut h = typed_in("db.us", &["users"]);
    press(&mut h, Key::Escape);
    assert!(!list_open(&h));
    assert_eq!(h.state().shell.script, "db.us");
    assert!(h.ctx.memory(|m| m.has_focus(editor_id(A))));
}

#[test]
fn enter_with_the_list_closed_inserts_a_newline() {
    let mut h = harness("x", &[]);
    caret_at(&mut h, 1);
    press(&mut h, Key::Enter);
    assert!(
        h.state().shell.script.starts_with("x\n"),
        "{:?}",
        h.state().shell.script
    );
    assert!(!list_open(&h));
}

#[test]
fn tab_with_the_list_closed_indents() {
    let mut h = harness("x", &[]);
    caret_at(&mut h, 0);
    press(&mut h, Key::Tab);
    assert_ne!(h.state().shell.script, "x");
    assert!(h.state().shell.script.ends_with('x'));
    assert!(h.state().shell.script.starts_with(char::is_whitespace));
}

#[test]
fn a_space_typed_while_the_list_is_open_is_typed_and_closes_it() {
    let mut h = typed_in("db.us", &["users"]);
    type_text(&mut h, " ");
    assert_eq!(h.state().shell.script, "db.us ");
    assert!(!list_open(&h));
}

#[test]
fn arrows_never_open_the_list() {
    let mut h = harness("db.us", &["users"]);
    caret_at(&mut h, 5);
    press(&mut h, Key::ArrowLeft);
    press(&mut h, Key::ArrowRight);
    assert!(!list_open(&h));
}

#[test]
fn ctrl_space_opens_the_list_on_an_empty_word() {
    let mut h = harness("", &[]);
    caret_at(&mut h, 0);
    h.key_press_modifiers(Modifiers::CTRL, Key::Space);
    h.run_steps(2);
    assert!(list_open(&h));
    assert_eq!(selected_item(&h).as_deref(), Some("db"));
    press(&mut h, Key::Enter);
    assert_eq!(h.state().shell.script, "db");
}

#[test]
fn a_pick_in_the_middle_of_a_word_does_not_double_it() {
    // db.u|sxs: the word `usxs` is not `users` yet, so the list is up.
    let mut h = harness("db.sxs.find()", &["users"]);
    caret_at(&mut h, 3);
    type_text(&mut h, "u");
    assert!(list_open(&h));
    press(&mut h, Key::Enter);
    assert_eq!(h.state().shell.script, "db.users.find()");
    assert_eq!(caret(&h), Some(8..8));
}

#[test]
fn clicking_an_item_picks_it() {
    let mut h = typed_in("db.us", &["usage", "users"]);
    h.get_by_role_and_label(Role::ListItem, "users").click();
    h.run_steps(3);
    assert_eq!(h.state().shell.script, "db.users");
    assert!(!list_open(&h));
    assert!(h.ctx.memory(|m| m.has_focus(editor_id(A))));
}

#[test]
fn the_list_is_a_labelled_list_of_named_items_with_the_selected_one_marked() {
    let mut h = typed_in("db.us", &["usage", "users"]);
    let items: Vec<_> = h
        .get_all_by_role(Role::ListItem)
        .map(|n| (n.accesskit_node().label(), n.accesskit_node().is_selected()))
        .collect();
    assert_eq!(
        items,
        [
            (Some("usage".to_string()), Some(true)),
            (Some("users".to_string()), Some(false))
        ]
    );
    press(&mut h, Key::ArrowDown);
    assert_eq!(selected_item(&h).as_deref(), Some("users"));
}

#[test]
fn the_list_stays_inside_the_window_near_its_bottom_edge() {
    let shell = ShellState::default();
    let size = egui::vec2(500.0, 260.0);
    let mut h = harness_sized(shell, &["a1", "a2", "a3", "a4", "a5", "a6"], size, 120.0);
    caret_at(&mut h, 0);
    type_text(&mut h, "\n\n\n\ndb.a");
    h.run_steps(2);
    assert!(list_open(&h));
    let rect = h
        .ctx
        .memory(|m| m.area_rect(popup_area_id(editor_id(A))))
        .expect("the list is drawn");
    let screen = h.ctx.screen_rect();
    assert!(screen.contains_rect(rect), "{rect:?} in {screen:?}");
    let caret_top = caret_rect_for_test(&h);
    assert!(
        rect.bottom() <= caret_top + 0.5,
        "flipped above the caret: {rect:?} {caret_top}"
    );
}

/// The top of the caret's line in screen points, from the editor's last layout.
fn caret_rect_for_test(h: &Harness<'static, Frame>) -> f32 {
    h.ctx
        .data(|d| d.get_temp::<egui::Rect>(editor_id(A).with(CARET_RECT)))
        .expect("the caret rect is remembered")
        .top()
}

#[test]
fn a_range_off_a_char_boundary_or_past_the_end_does_not_panic() {
    assert_eq!(applied("é", &(1..9), "x"), ("x".into(), 1));
    assert_eq!(applied("ab", &(5..9), "x"), ("abx".into(), 3));
}

#[test]
fn enter_after_a_complete_word_is_a_newline() {
    for word in ["show dbs", "show collections", "it", "use"] {
        let mut h = typed_in(word, &[]);
        press(&mut h, Key::Enter);
        assert_eq!(h.state().shell.script, format!("{word}\n"), "{word:?}");
    }
}

#[test]
fn tab_after_a_complete_collection_name_indents() {
    let mut h = typed_in("db.users", &["users", "user_prefs"]);
    press(&mut h, Key::Tab);
    let script = &h.state().shell.script;
    assert!(
        script.starts_with("db.users") && script.len() > 8,
        "{script:?}"
    );
    assert!(script[8..].chars().all(char::is_whitespace), "{script:?}");
}

#[test]
fn shift_tab_and_shift_enter_pass_through_an_open_list() {
    let mut h = typed_in("db.us", &["users"]);
    press_with(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(h.state().shell.script, "db.us", "Shift+Tab picks nothing");
    let mut h = typed_in("db.us", &["users"]);
    press_with(&mut h, Modifiers::SHIFT, Key::Enter);
    assert!(
        !h.state().shell.script.contains("users"),
        "{:?}",
        h.state().shell.script
    );
}

#[test]
fn a_paste_or_a_multi_character_text_does_not_open_the_list() {
    let mut h = harness("", &["users"]);
    caret_at(&mut h, 0);
    h.input_mut().events.push(Event::Paste("db.us".into()));
    h.run_steps(2);
    assert_eq!(h.state().shell.script, "db.us");
    assert!(!list_open(&h));
    let mut h = harness("", &["users"]);
    caret_at(&mut h, 0);
    h.input_mut().events.push(Event::Text("db.us".into()));
    h.run_steps(2);
    assert_eq!(h.state().shell.script, "db.us");
    assert!(!list_open(&h));
}

#[test]
fn undo_after_a_pick_brings_back_the_text_before_it() {
    let mut h = typed_in("db.us", &["users"]);
    press(&mut h, Key::Enter);
    assert_eq!(h.state().shell.script, "db.users");
    press_with(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(h.state().shell.script, "db.us");
}

#[test]
fn a_script_replaced_by_the_state_closes_the_open_list() {
    let mut h = typed_in("db.us", &["users"]);
    assert!(list_open(&h));
    // Like a history recall: the state puts other text in the editor, and Enter lands in the
    // same frame.
    let recalled = h.state().shell.edited("é".into(), None);
    h.state_mut().shell = recalled;
    press(&mut h, Key::Enter);
    assert!(!list_open(&h));
    let script = &h.state().shell.script;
    assert!(
        !script.contains("users"),
        "nothing picked into it: {script:?}"
    );
}

#[test]
fn every_kind_marker_and_the_list_name_are_translated() {
    let kinds = [
        SuggestionKind::Collection,
        SuggestionKind::DbMethod,
        SuggestionKind::CollectionMethod,
        SuggestionKind::Operator,
        SuggestionKind::Word,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for key in kinds
        .map(kind_key)
        .into_iter()
        .chain(["shell.complete.list_label"])
    {
        let text = t(key);
        assert!(!text.is_empty() && text != key, "{key} is missing");
        assert!(seen.insert(text), "{key} repeats another marker");
    }
}
