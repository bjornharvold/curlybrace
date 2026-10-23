//! The index builder in the accesskit tree: every control is named, reachable with the keyboard
//! and reports the right edit; Create is disabled with a stated reason while the builder is
//! invalid or the connection is read-only. Fast, offline, no GPU.
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::index_builder::BUILD_WARNING_THRESHOLD;
use curlybrace_ui::viewmodel::{BuilderEdit, IndexBuilderState, IndexesAction};
use curlybrace_ui::views::index_builder::{BuilderContext, show_index_builder};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn named(key: &str, n: &str) -> String {
    I18n::english().t_with(key, &[("n", n)])
}

/// The builder, the actions it reported, and what the connection allows.
struct Frame {
    builder: IndexBuilderState,
    seen: Vec<IndexesAction>,
    read_only: bool,
    busy: bool,
}

fn builder() -> IndexBuilderState {
    IndexBuilderState::new(Namespace::new("shop", "orders"))
}

/// `a` then `b`: two keys, so every reorder control has somewhere to go.
fn two_keys() -> IndexBuilderState {
    builder().with_field(0, "a").adding_key().with_field(1, "b")
}

fn open(builder: IndexBuilderState) -> Harness<'static, Frame> {
    open_with(builder, false, false)
}

/// Draws the modal; an edit is applied to the state the way the reducer would, so typing and
/// clicking can be followed over several frames.
fn open_with(builder: IndexBuilderState, read_only: bool, busy: bool) -> Harness<'static, Frame> {
    let frame = Frame {
        builder,
        seen: Vec::new(),
        read_only,
        busy,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 900.0))
        .build_state(
            |ctx, frame: &mut Frame| {
                let context = BuilderContext {
                    connection: "local",
                    read_only: frame.read_only,
                    busy: frame.busy,
                };
                let action = show_index_builder(ctx, &frame.builder, &context, &I18n::english());
                if let Some(IndexesAction::Builder(edit)) = &action {
                    frame.builder = frame.builder.clone().edited(edit.clone());
                }
                frame.seen.extend(action);
            },
            frame,
        );
    // The modal settles over a few frames (its size is unknown on the first); clicks aim at
    // where its controls end up.
    harness.run_steps(10);
    harness
}

fn count(harness: &Harness<'static, Frame>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

fn button<'a>(harness: &'a Harness<'static, Frame>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::Button, label)
}

fn seen(harness: &Harness<'static, Frame>) -> Vec<IndexesAction> {
    harness.state().seen.clone()
}

/// The name of a control: its own label, or the label of the caption it is labelled by.
fn name_of(node: &egui_kittest::Node<'_>) -> Option<String> {
    let own = node.accesskit_node().label();
    own.or_else(|| {
        node.accesskit_node()
            .labelled_by()
            .next()
            .and_then(|caption| caption.label())
    })
}

#[test]
fn builder_controls_are_labelled_and_keyboard_reachable() {
    let mut harness = open(two_keys());
    let state_labels = [
        t("indexes.builder.title"),
        t("indexes.builder.keys_heading"),
        t("indexes.builder.options_heading"),
        t("indexes.builder.tip"),
        t("indexes.builder.preview_heading"),
    ];
    for label in &state_labels {
        harness.get_by_label(label);
    }
    // Every text box, picker and button has a name.
    let roles = [
        Role::TextInput,
        Role::MultilineTextInput,
        Role::ComboBox,
        Role::Button,
        Role::CheckBox,
    ];
    let mut unnamed = Vec::new();
    for role in roles {
        for node in harness.query_all_by_role(role) {
            if name_of(&node).is_none_or(|n| n.trim().is_empty()) {
                unnamed.push(format!("{role:?}"));
            }
        }
    }
    assert!(unnamed.is_empty(), "controls without a name: {unnamed:?}");

    // Tab reaches each of them.
    let mut reached = Vec::new();
    for _ in 0..40 {
        harness.key_press(egui::Key::Tab);
        harness.run();
        let focused = harness
            .root()
            .children_recursive()
            .find(|n| n.accesskit_node().is_focused());
        reached.extend(focused.and_then(|n| name_of(&n)));
    }
    let expected = [
        named("indexes.builder.key_field_n", "1"),
        named("indexes.builder.key_field_n", "2"),
        named("indexes.builder.key_type_n", "1"),
        named("indexes.builder.key_type_n", "2"),
        named("indexes.builder.move_down_n", "1"),
        named("indexes.builder.move_up_n", "2"),
        named("indexes.builder.remove_key_n", "1"),
        named("indexes.builder.remove_key_n", "2"),
        t("indexes.builder.add_key"),
        t("indexes.builder.name"),
        t("indexes.builder.ttl"),
        t("indexes.builder.unique"),
        t("indexes.builder.sparse"),
        t("indexes.builder.hidden"),
        t("indexes.builder.collation"),
        t("indexes.builder.partial_filter"),
        t("indexes.builder.create"),
        t("common.cancel"),
    ];
    for label in expected {
        assert!(reached.contains(&label), "{label} not reached: {reached:?}");
    }
}

#[test]
fn the_first_and_last_keys_cannot_move_past_the_ends() {
    let harness = open(two_keys());
    let up_first = named("indexes.builder.move_up_n", "1");
    let down_last = named("indexes.builder.move_down_n", "2");
    assert!(button(&harness, &up_first).accesskit_node().is_disabled());
    assert!(button(&harness, &down_last).accesskit_node().is_disabled());
    let down_first = named("indexes.builder.move_down_n", "1");
    assert!(!button(&harness, &down_first).accesskit_node().is_disabled());
}

#[test]
fn the_controls_report_their_edits() {
    let mut harness = open(two_keys());
    button(&harness, &named("indexes.builder.move_down_n", "1")).click();
    harness.run_steps(3);
    button(&harness, &named("indexes.builder.remove_key_n", "1")).click();
    harness.run_steps(3);
    button(&harness, &t("indexes.builder.add_key")).click();
    harness.run_steps(3);
    harness.get_by_label(&t("indexes.builder.unique")).click();
    harness.run_steps(3);
    assert_eq!(
        seen(&harness),
        vec![
            IndexesAction::Builder(BuilderEdit::MoveKey { from: 0, to: 1 }),
            IndexesAction::Builder(BuilderEdit::RemoveKey(0)),
            IndexesAction::Builder(BuilderEdit::AddKey),
            IndexesAction::Builder(BuilderEdit::SetUnique(true)),
        ]
    );
}

#[test]
fn typing_a_field_and_picking_a_type_edit_the_builder() {
    let mut harness = open(builder());
    let field_name = named("indexes.builder.key_field_n", "1");
    let field = harness.get_by_role_and_label(Role::TextInput, &field_name);
    field.focus();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::TextInput, &field_name)
        .type_text("address.city");
    harness.run_steps(4);
    assert_eq!(harness.state().builder.keys[0].field, "address.city");
    assert_eq!(harness.state().builder.name, "address.city_1");

    harness
        .get_by_role_and_label(Role::ComboBox, &named("indexes.builder.key_type_n", "1"))
        .click();
    harness.run_steps(3);
    harness
        .get_by_label(&t("indexes.builder.kind_geo_sphere"))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().builder.keys[0].kind, KeyKind::Geo2dSphere);
    // Only the five builder kinds are offered, hashed and 2d never.
    harness
        .get_by_role_and_label(Role::ComboBox, &named("indexes.builder.key_type_n", "1"))
        .click();
    harness.run_steps(3);
    for key in [
        "indexes.builder.kind_asc",
        "indexes.builder.kind_desc",
        "indexes.builder.kind_text",
        "indexes.builder.kind_wildcard",
    ] {
        harness.get_by_label(&t(key));
    }
    for key in [
        "indexes.builder.kind_hashed",
        "indexes.builder.kind_geo_flat",
    ] {
        assert_eq!(count(&harness, &t(key)), 0, "{key}");
    }
}

#[test]
fn create_is_disabled_with_the_reason_while_the_builder_is_invalid() {
    // One empty key row.
    let harness = open(builder());
    assert!(
        button(&harness, &t("indexes.builder.create"))
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label(&t("indexes.builder.problem_empty_field"));

    let none = open(builder().removing_key(0));
    assert!(
        button(&none, &t("indexes.builder.create"))
            .accesskit_node()
            .is_disabled()
    );
    none.get_by_label(&t("indexes.builder.problem_no_keys"));

    let twice = open(builder().with_field(0, "a").adding_key().with_field(1, "a"));
    let text = I18n::english().t_with("indexes.builder.problem_duplicate", &[("field", "a")]);
    twice.get_by_label(&text);

    let ttl = open(two_keys().with_ttl(Some(60)));
    ttl.get_by_label(&t("indexes.builder.problem_ttl_keys"));

    let filter = open(
        builder()
            .with_field(0, "a")
            .with_partial_filter_text("{ nope"),
    );
    filter.get_by_label(&t("indexes.builder.problem_partial_filter"));
}

#[test]
fn create_is_enabled_for_a_valid_builder_and_reports_create() {
    let mut harness = open(builder().with_field(0, "a"));
    let create = t("indexes.builder.create");
    assert!(!button(&harness, &create).accesskit_node().is_disabled());
    for key in [
        "indexes.builder.problem_empty_field",
        "indexes.builder.problem_no_keys",
    ] {
        assert_eq!(count(&harness, &t(key)), 0);
    }
    button(&harness, &create).click();
    harness.run_steps(3);
    assert_eq!(seen(&harness), vec![IndexesAction::Create]);

    button(&harness, &t("common.cancel")).click();
    harness.run_steps(3);
    assert_eq!(seen(&harness).last(), Some(&IndexesAction::CancelBuilder));
}

#[test]
fn the_preview_shows_the_command_and_follows_the_keys() {
    let mut harness = open(builder().with_field(0, "email"));
    let preview = harness.state().builder.preview_json();
    assert!(preview.contains("createIndexes") && preview.contains("email_1"));
    let heading = t("indexes.builder.preview_heading");
    let node = harness.get_by_role_and_label(Role::MultilineTextInput, &heading);
    assert_eq!(
        node.accesskit_node().value().as_deref(),
        Some(preview.as_str())
    );
    // The preview is read-only: typing into it changes nothing.
    node.focus();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::MultilineTextInput, &heading)
        .type_text("x");
    harness.run_steps(3);
    assert_eq!(harness.state().builder, builder().with_field(0, "email"));
}

#[test]
fn the_build_warning_shows_above_create_only_for_a_large_collection() {
    let warning = |documents: u64| {
        I18n::english().t_with(
            "indexes.builder.warning_large",
            &[("documents", &documents.to_string())],
        )
    };
    let large = BUILD_WARNING_THRESHOLD + 1;
    let harness = open(builder().with_field(0, "a").with_estimate(large));
    harness.get_by_label(&warning(large));
    // And it does not stand in the way of Create.
    assert!(
        !button(&harness, &t("indexes.builder.create"))
            .accesskit_node()
            .is_disabled()
    );

    for documents in [0, BUILD_WARNING_THRESHOLD] {
        let harness = open(builder().with_field(0, "a").with_estimate(documents));
        assert_eq!(count(&harness, &warning(documents)), 0, "{documents}");
    }
    // No estimate at all: no warning, Create unaffected.
    let harness = open(builder().with_field(0, "a"));
    assert!(
        !button(&harness, &t("indexes.builder.create"))
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn a_read_only_connection_disables_create_and_says_why() {
    let harness = open_with(builder().with_field(0, "a"), true, false);
    assert!(
        button(&harness, &t("indexes.builder.create"))
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label(&t("connection.read_only_reason"));
}

#[test]
fn create_waits_while_another_write_is_in_flight() {
    let harness = open_with(builder().with_field(0, "a"), false, true);
    assert!(
        button(&harness, &t("indexes.builder.create"))
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label(&t("indexes.builder.busy"));
}

#[test]
fn escape_closes_an_untouched_builder_but_keeps_one_with_input() {
    let mut harness = open(builder());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert_eq!(seen(&harness), vec![IndexesAction::CancelBuilder]);

    let mut typed = open(builder().with_field(0, "a"));
    typed.key_press(egui::Key::Escape);
    typed.run_steps(3);
    assert!(
        seen(&typed).is_empty(),
        "typed input is not thrown away by Escape"
    );
}

#[test]
fn hashed_and_2d_keys_of_an_existing_index_show_their_type() {
    let hashed = builder().with_field(0, "h").with_kind(0, KeyKind::Hashed);
    let harness = open(hashed);
    let label = named("indexes.builder.key_type_n", "1");
    let combo = harness.get_by_role_and_label(Role::ComboBox, &label);
    assert_eq!(
        combo.accesskit_node().value().as_deref(),
        Some(t("indexes.builder.kind_hashed").as_str())
    );
}
