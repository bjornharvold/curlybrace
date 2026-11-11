//! The Shell tab's Table and Tree modes in the accesskit tree: the `Text | Table | Tree` switch,
//! the reason a mode cannot show a result (with a way back to Text), the table's header and its
//! virtualised rows, the tree's expandable rows, the notes about what is not shown, and the cache
//! that keeps a repaint from rebuilding the rows. Real `ShellState`s and real `ShellResult`s,
//! no mocks. Fast, offline, no GPU.
use bson::{Bson, Document, doc};
use curlybrace_core::shell::{DocKind, Outcome, ShellResult};
use curlybrace_ui::I18n;
use curlybrace_ui::app::shell_actions::ShellAction;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::shell::ShellState;
use curlybrace_ui::viewmodel::shell_result::OutputMode;
use curlybrace_ui::views::shell::{ShellViewContext, ShellViewEvent, show_shell};
use curlybrace_ui::views::shell_result_cache::{CELL_MAX_CHARS, cached_table, cached_tree};
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::sync::Arc;
use uuid::Uuid;

const SESSION: Uuid = Uuid::from_u128(21);
const NOW: i64 = 1_700_000_000;
const SMALL: egui::Vec2 = egui::vec2(800.0, 500.0);

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

struct Frame {
    shell: ShellState,
    actions: Vec<ShellAction>,
}

/// What the reducer does to the shell for the result actions.
fn applied(shell: &ShellState, action: &ShellAction) -> ShellState {
    match action {
        ShellAction::SetOutputMode(mode) => shell.mode_set(*mode),
        ShellAction::TreeToggle(doc, path) => shell.tree_toggled(*doc, path.clone()),
        ShellAction::TreeExpandAll => shell.tree_all_set(true),
        ShellAction::TreeCollapseAll => shell.tree_all_set(false),
        _ => shell.clone(),
    }
}

fn harness_sized(shell: ShellState, size: egui::Vec2, dark: bool) -> Harness<'static, Frame> {
    let i18n = I18n::english();
    let mut harness = Harness::builder().with_size(size).build_ui_state(
        move |ui, frame: &mut Frame| {
            let ctx = ShellViewContext {
                session: SESSION,
                connection: "local",
                badge: ConnectionBadge::default(),
                connected: true,
                now_secs: NOW,
            };
            for event in show_shell(ui, &frame.shell, ctx, &i18n) {
                if let ShellViewEvent::Action(action) = event {
                    frame.shell = applied(&frame.shell, &action);
                    frame.actions.push(action);
                }
            }
        },
        Frame {
            shell,
            actions: Vec::new(),
        },
    );
    theme::install(&harness.ctx);
    if dark {
        harness.ctx.set_visuals(egui::Visuals::dark());
    }
    harness.run_steps(4);
    harness
}

fn harness(shell: ShellState) -> Harness<'static, Frame> {
    harness_sized(shell, egui::vec2(1000.0, 800.0), false)
}

/// A shell whose first run returned `result` and ended cleanly, shown in `mode`.
fn with_result(result: ShellResult, mode: OutputMode) -> ShellState {
    ShellState::default()
        .edited("db.a.find()".into(), None)
        .run_requested()
        .0
        .result_arrived(1, result)
        .finished(1, Outcome::Exited(0))
        .mode_set(mode)
}

fn cursor(docs: Vec<Document>, more: bool) -> ShellResult {
    ShellResult::Documents {
        kind: DocKind::Cursor,
        docs,
        more,
        total: None,
    }
}

fn people() -> Vec<Document> {
    vec![
        doc! { "_id": 1_i32, "name": "Ada" },
        doc! { "_id": 2_i32, "name": "Grace" },
    ]
}

fn numbered(n: usize) -> Vec<Document> {
    (0..n)
        .map(|i| doc! { "_id": i as i32, "name": format!("doc {i}") })
        .collect()
}

/// Whether the mode's radio button is disabled, and whether it is the selected one.
fn radio(h: &Harness<'static, Frame>, key: &str) -> (bool, bool) {
    let label = t(key);
    let node = h.get_by_role_and_label(Role::RadioButton, &label);
    let disabled = node.accesskit_node().is_disabled();
    let selected = node.accesskit_node().toggled() == Some(Toggled::True);
    (disabled, selected)
}

fn row_toggle(key: &str, state: &str) -> String {
    tw(
        "shell.result.row_toggle",
        &[("key", key), ("state", &t(state))],
    )
}

#[test]
fn the_mode_switch_has_three_labelled_buttons() {
    let h = harness(with_result(cursor(people(), false), OutputMode::Text));
    let name = t("shell.result.mode_label");
    h.get_by_role_and_label(Role::Label, &name);
    let group = h.get_by_role_and_label(Role::RadioGroup, &name);
    for key in [
        "shell.result.mode.text",
        "shell.result.mode.table",
        "shell.result.mode.tree",
    ] {
        group.get_by_role_and_label(Role::RadioButton, &t(key));
    }
    assert_eq!(radio(&h, "shell.result.mode.text"), (false, true));
    assert_eq!(radio(&h, "shell.result.mode.table"), (false, false));
    assert_eq!(radio(&h, "shell.result.mode.tree"), (false, false));
}

#[test]
fn unavailable_modes_are_disabled_with_their_reason() {
    let h = harness(ShellState::default());
    assert!(radio(&h, "shell.result.mode.table").0);
    assert!(radio(&h, "shell.result.mode.tree").0);
    assert!(!radio(&h, "shell.result.mode.text").0);
    let reason = t("shell.result.unavailable.no_value");
    assert!(h.query_by_label(&reason).is_none(), "shown before hover");
    h.get_by_role_and_label(Role::RadioButton, &t("shell.result.mode.table"))
        .hover();
    let mut h = h;
    h.run_steps(3);
    h.get_by_label(&reason);

    let values = ShellResult::Values {
        values: vec![Bson::Int32(1)],
        more: false,
        total: 1,
    };
    let h = harness(with_result(values, OutputMode::Text));
    assert!(radio(&h, "shell.result.mode.table").0);
    assert!(!radio(&h, "shell.result.mode.tree").0);
}

#[test]
fn switching_modes_emits_set_output_mode() {
    let mut h = harness(with_result(cursor(people(), false), OutputMode::Text));
    h.get_by_role_and_label(Role::RadioButton, &t("shell.result.mode.table"))
        .click();
    h.run_steps(3);
    let actions = &h.state().actions;
    assert!(actions.contains(&ShellAction::SetOutputMode(OutputMode::Table)));
    assert!(radio(&h, "shell.result.mode.table").1);
}

#[test]
fn selecting_table_shows_a_column_header_and_rows() {
    let h = harness(with_result(cursor(people(), false), OutputMode::Table));
    for label in ["_id", "name", "Ada", "Grace", "1", "2"] {
        h.get_by_label(label);
    }
    h.get_by_label(&tw("shell.result.row_count", &[("count", "2")]));
}

#[test]
fn selecting_tree_shows_expandable_rows() {
    let docs = vec![doc! { "name": "Ada", "address": { "city": "London" } }];
    let mut h = harness(with_result(cursor(docs, false), OutputMode::Tree));
    h.get_by_label(&tw("doc.header", &[("n", "1")]));
    h.get_by_label("Ada");
    let closed = row_toggle("address", "shell.result.collapsed");
    let node = h.get_by_role_and_label(Role::Button, &closed);
    assert_eq!(node.accesskit_node().data().is_expanded(), Some(false));
    assert!(h.query_by_label("London").is_none());
    node.click();
    h.run_steps(3);
    let toggle = ShellAction::TreeToggle(0, vec!["address".to_string()]);
    assert!(h.state().actions.contains(&toggle));
    let open = row_toggle("address", "shell.result.expanded");
    let node = h.get_by_role_and_label(Role::Button, &open);
    assert_eq!(node.accesskit_node().data().is_expanded(), Some(true));
    h.get_by_label("London");
}

#[test]
fn expand_all_and_collapse_all_emit_their_actions() {
    let docs = vec![doc! { "a": { "b": { "c": 1_i32 } } }];
    let mut h = harness(with_result(cursor(docs, false), OutputMode::Tree));
    h.get_by_role_and_label(Role::Button, &t("shell.result.expand_all"))
        .click();
    h.run_steps(3);
    assert!(h.state().actions.contains(&ShellAction::TreeExpandAll));
    h.get_by_label("c");
    h.get_by_role_and_label(Role::Button, &t("shell.result.collapse_all"))
        .click();
    h.run_steps(3);
    assert!(h.state().actions.contains(&ShellAction::TreeCollapseAll));
    assert!(h.query_by_label("c").is_none());
}

#[test]
fn a_capped_expand_all_is_said() {
    let nested: Document = (0..30)
        .map(|i| (format!("f{i}"), Bson::Document(doc! { "x": 1_i32 })))
        .collect();
    let docs = vec![nested; 200];
    let shell = with_result(cursor(docs, false), OutputMode::Tree).tree_all_set(true);
    assert!(shell.result.tree.capped);
    let h = harness(shell);
    h.get_by_label(&t("shell.result.expand_capped"));
}

#[test]
fn table_shows_the_reason_when_the_result_is_not_documents() {
    let scalar = ShellResult::Scalar {
        value: Bson::Int32(42),
        undefined: false,
    };
    let mut h = harness(with_result(scalar, OutputMode::Table));
    h.get_by_label(&t("shell.result.unavailable.not_documents"));
    h.get_by_role_and_label(Role::Button, &t("shell.result.show_text"))
        .click();
    h.run_steps(3);
    let actions = &h.state().actions;
    assert!(actions.contains(&ShellAction::SetOutputMode(OutputMode::Text)));
    assert!(
        h.query_by_label(&t("shell.result.unavailable.not_documents"))
            .is_none()
    );
}

#[test]
fn every_reason_is_said_in_words() {
    let too_big = with_result(ShellResult::TooBig { bytes: 5 << 20 }, OutputMode::Table);
    let h = harness(too_big);
    let size = tw("indexes.size_mib", &[("n", "5.0")]);
    h.get_by_label(&tw("shell.result.unavailable.too_big", &[("size", &size)]));

    let unsupported = ShellResult::Unsupported { type_name: None };
    let h = harness(with_result(unsupported, OutputMode::Tree));
    h.get_by_label(&t("shell.result.unavailable.unsupported"));

    let pending = ShellState::default()
        .edited("db.a.find()".into(), None)
        .run_requested()
        .0
        .mode_set(OutputMode::Table);
    let h = harness(pending.clone());
    h.get_by_label(&t("shell.result.unavailable.pending"));

    let failed = pending.finished(1, Outcome::ScriptErrors);
    let h = harness(failed);
    h.get_by_label(&t("shell.result.unavailable.run_failed"));
}

#[test]
fn the_more_in_cursor_note_is_announced() {
    let h = harness(with_result(cursor(numbered(20), true), OutputMode::Table));
    let note = tw("shell.result.more_in_cursor", &[("count", "20")]);
    h.get_by_label_contains(&note);
    let h = harness(with_result(cursor(numbered(20), true), OutputMode::Tree));
    h.get_by_label_contains(&note);
}

#[test]
fn showing_of_note_for_a_long_array() {
    let array = ShellResult::Documents {
        kind: DocKind::Array,
        docs: numbered(200),
        more: true,
        total: Some(1000),
    };
    let h = harness(with_result(array, OutputMode::Table));
    let note = tw(
        "shell.result.showing_of",
        &[("shown", "200"), ("total", "1000")],
    );
    // The notes are one label: "Rows: 200 · Showing 200 of 1000."
    let rows = tw("shell.result.row_count", &[("count", "200")]);
    let joined = [rows, note].join(&t("shell.result.note_separator"));
    h.get_by_label(&joined);
}

fn wide(columns: usize) -> Document {
    (0..columns)
        .map(|i| (format!("field_{i}"), Bson::Int32(i as i32)))
        .collect()
}

#[test]
fn more_columns_note_for_a_wide_table() {
    let h = harness(with_result(
        cursor(vec![wide(60)], false),
        OutputMode::Table,
    ));
    h.get_by_label_contains(&tw("shell.result.more_columns", &[("count", "10")]));
    let h = harness(with_result(
        cursor(vec![wide(40)], false),
        OutputMode::Table,
    ));
    assert!(
        h.query_by_label_contains(&tw("shell.result.more_columns", &[("count", "")]))
            .is_none()
    );
}

#[test]
fn a_result_of_200_documents_renders_only_visible_rows() {
    let h = harness(with_result(cursor(numbered(200), false), OutputMode::Table));
    h.get_by_label("doc 0");
    assert!(
        h.query_by_label("doc 199").is_none(),
        "every row was laid out"
    );
    let h = harness(with_result(cursor(numbered(200), false), OutputMode::Tree));
    h.get_by_label("doc 0");
    assert!(
        h.query_by_label("doc 199").is_none(),
        "every row was laid out"
    );
}

#[test]
fn repainting_does_not_recompute_the_cached_rows() {
    let mut h = harness(with_result(cursor(people(), false), OutputMode::Table));
    let first = cached_table(&h.ctx, SESSION).expect("table cached");
    h.run_steps(5);
    let again = cached_table(&h.ctx, SESSION).expect("table cached");
    assert!(Arc::ptr_eq(&first, &again), "the table was rebuilt");

    let docs = vec![doc! { "a": { "b": 1_i32 } }];
    let mut h = harness(with_result(cursor(docs, false), OutputMode::Tree));
    let first = cached_tree(&h.ctx, SESSION).expect("tree cached");
    h.run_steps(5);
    let again = cached_tree(&h.ctx, SESSION).expect("tree cached");
    assert!(Arc::ptr_eq(&first, &again), "the tree was rebuilt");
    let closed = row_toggle("a", "shell.result.collapsed");
    h.get_by_role_and_label(Role::Button, &closed).click();
    h.run_steps(3);
    let toggled = cached_tree(&h.ctx, SESSION).expect("tree cached");
    assert!(!Arc::ptr_eq(&first, &toggled), "a toggle kept stale rows");
}

/// Every text that contains `part` is short: the value was cut before it was laid out. (A
/// label's text is its accesskit value; a widget's is its label.)
fn assert_cut(h: &Harness<'static, Frame>, part: &str) {
    let labels: Vec<String> = h
        .query_all_by_label_contains(part)
        .filter_map(|node| {
            let node = node.accesskit_node();
            node.label().or_else(|| node.value())
        })
        .collect();
    assert!(!labels.is_empty(), "no cell shows the value");
    for label in labels {
        assert!(
            label.chars().count() <= CELL_MAX_CHARS + 8,
            "a label of {} chars",
            label.chars().count()
        );
    }
}

#[test]
fn a_one_mebibyte_cell_is_truncated_not_laid_out() {
    let huge = "x".repeat(1 << 20);
    let docs = vec![doc! { "_id": 1_i32, "blob": huge }];
    let h = harness(with_result(cursor(docs.clone(), false), OutputMode::Table));
    assert_cut(&h, "xxxxxxxx");
    let h = harness(with_result(cursor(docs, false), OutputMode::Tree));
    assert_cut(&h, "xxxxxxxx");
}

#[test]
fn an_empty_result_says_so() {
    for mode in [OutputMode::Table, OutputMode::Tree] {
        let h = harness(with_result(cursor(Vec::new(), false), mode));
        h.get_by_label(&t("shell.result.empty"));
    }
}

#[test]
fn tree_values_show_each_element() {
    let values = ShellResult::Values {
        values: vec![
            Bson::Int32(7),
            Bson::String("two".into()),
            Bson::Boolean(true),
        ],
        more: false,
        total: 3,
    };
    let h = harness(with_result(values, OutputMode::Tree));
    h.get_by_label(&t("shell.result.values_title"));
    for label in ["0", "1", "2", "two", "true"] {
        h.get_by_label(label);
    }
}

#[test]
fn an_object_is_one_row_of_the_table_and_one_tree() {
    let object = ShellResult::Object {
        type_name: None,
        doc: doc! { "ok": 1_i32, "host": "db1" },
    };
    let h = harness(with_result(object.clone(), OutputMode::Table));
    h.get_by_label("host");
    h.get_by_label("db1");
    let h = harness(with_result(object, OutputMode::Tree));
    h.get_by_label(&tw("doc.header", &[("n", "1")]));
    h.get_by_label("db1");
}

/// Dark, narrow, 40 columns of 200 rows: the notes wrap inside the window instead of running
/// off its right edge.
#[test]
fn a_wide_long_table_at_a_narrow_window_keeps_its_notes_inside() {
    let docs: Vec<Document> = (0..200).map(|_| wide(40)).collect();
    let array = ShellResult::Documents {
        kind: DocKind::Array,
        docs,
        more: true,
        total: Some(5000),
    };
    let h = harness_sized(with_result(array, OutputMode::Table), SMALL, true);
    let note = tw(
        "shell.result.showing_of",
        &[("shown", "200"), ("total", "5000")],
    );
    let rect = h.get_by_label_contains(&note).rect();
    assert!(rect.right() <= SMALL.x, "the note is clipped: {rect:?}");
    h.get_by_label("field_0");
}

#[test]
fn a_disabled_mode_describes_its_reason_to_assistive_technology() {
    let h = harness(ShellState::default());
    let reason = t("shell.result.unavailable.no_value");
    for key in ["shell.result.mode.table", "shell.result.mode.tree"] {
        let label = t(key);
        let node = h.get_by_role_and_label(Role::RadioButton, &label);
        assert_eq!(node.accesskit_node().description(), Some(reason.clone()));
    }
    let text = t("shell.result.mode.text");
    let node = h.get_by_role_and_label(Role::RadioButton, &text);
    assert_eq!(node.accesskit_node().description(), None);
}

#[test]
fn an_empty_array_of_values_says_so() {
    let values = ShellResult::Values {
        values: Vec::new(),
        more: false,
        total: 0,
    };
    let h = harness(with_result(values, OutputMode::Tree));
    h.get_by_label(&t("shell.result.empty_values"));
    assert!(h.query_by_label(&t("shell.result.values_title")).is_none());
}

#[test]
fn a_long_key_leaves_the_value_in_view() {
    let key = "k".repeat(1_000);
    let docs = vec![doc! { key.as_str(): "v1" }];
    let h = harness_sized(
        with_result(cursor(docs, false), OutputMode::Tree),
        SMALL,
        false,
    );
    let value = h.get_by_label("v1").rect();
    assert!(
        value.right() <= SMALL.x,
        "the value is pushed out: {value:?}"
    );
    let first = h.get_by_label("v1").rect().center().y;
    let shown_key = h.get_by_label_contains("kkkk").rect();
    assert!(
        (shown_key.center().y - first).abs() < 1.0,
        "the row wrapped"
    );
}

#[test]
fn a_new_run_viewed_as_text_drops_the_old_rows() {
    let mut h = harness(with_result(cursor(people(), false), OutputMode::Table));
    h.state_mut().shell = h.state().shell.mode_set(OutputMode::Tree);
    h.run_steps(3);
    assert!(cached_table(&h.ctx, SESSION).is_some());
    assert!(cached_tree(&h.ctx, SESSION).is_some());
    let (next, request) = h.state().shell.mode_set(OutputMode::Text).run_requested();
    let run_id = request.expect("a second run").run_id;
    h.state_mut().shell = next
        .result_arrived(run_id, cursor(numbered(3), false))
        .finished(run_id, Outcome::Exited(0));
    h.run_steps(3);
    assert!(
        cached_table(&h.ctx, SESSION).is_none(),
        "the table was kept"
    );
    assert!(cached_tree(&h.ctx, SESSION).is_none(), "the tree was kept");
}
