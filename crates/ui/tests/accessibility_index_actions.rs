//! The per-index buttons and the small modals of the index writes in the accesskit tree: every
//! control is named and reachable with the keyboard, write controls are disabled with a stated
//! reason (read-only, a write in flight, the `_id_` index), and the drop needs the exact name
//! typed. Fast, offline, no GPU.
use bson::doc;
use curlybrace_core::indexes::{IndexOptions, IndexSpec, KeyKind};
use curlybrace_core::store::{CollectionIndex, Namespace};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use curlybrace_ui::I18n;
use curlybrace_ui::app::sampling_settings::SamplingSettings;
use curlybrace_ui::viewmodel::{
    ConnectionBadge, DropConfirm, DropOld, IndexWrite, IndexesAction, IndexesState, TtlPrompt,
};
use curlybrace_ui::views::index_dialogs::{
    DialogContext, show_drop_confirm, show_drop_old, show_ttl_prompt,
};
use curlybrace_ui::views::indexes::show_indexes;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const NOW: i64 = 1_800_000_000;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn listed(spec: IndexSpec) -> CollectionIndex {
    CollectionIndex {
        spec,
        size_bytes: 4_096,
    }
}

fn plain(name: &str) -> IndexSpec {
    mock_index_spec_named(name, &[("a", KeyKind::Asc)])
}

fn expiring() -> IndexSpec {
    IndexSpec {
        options: IndexOptions {
            expire_after_seconds: Some(60),
            ..IndexOptions::default()
        },
        ..plain("ttl_1")
    }
}

fn id_index() -> IndexSpec {
    mock_index_spec_named("_id_", &[("_id", KeyKind::Asc)])
}

fn tab(selected: &str, writing: Option<IndexWrite>) -> IndexesState {
    IndexesState {
        ns: Some(Namespace::new("shop", "orders")),
        indexes: vec![listed(id_index()), listed(plain("a_1")), listed(expiring())],
        selected: Some(selected.into()),
        writing,
        ..IndexesState::default()
    }
}

type Frame = (IndexesState, Vec<IndexesAction>);

fn open_tab(state: IndexesState, read_only: bool) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 700.0))
        .build_ui_state(
            move |ui, (state, seen): &mut Frame| {
                seen.extend(show_indexes(
                    ui,
                    state,
                    read_only,
                    &I18n::english(),
                    NOW,
                    &SamplingSettings::default(),
                ));
            },
            (state, Vec::new()),
        );
    harness.run_steps(4);
    harness
}

fn button<'a>(harness: &'a Harness<'static, Frame>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::Button, label)
}

fn disabled(harness: &Harness<'static, Frame>, label: &str) -> bool {
    button(harness, label).accesskit_node().is_disabled()
}

const WRITES: [&str; 3] = [
    "indexes.action_hide",
    "indexes.action_edit_copy",
    "indexes.action_drop",
];

/// Everything on screen that can be named, in tree order.
fn names(harness: &Harness<'static, impl Sized>) -> Vec<String> {
    harness
        .root()
        .children_recursive()
        .filter_map(|n| n.accesskit_node().label())
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn the_write_controls_report_their_actions() {
    let mut harness = open_tab(tab("a_1", None), false);
    for key in [
        "indexes.action_hide",
        "indexes.action_edit_copy",
        "indexes.action_drop",
    ] {
        assert!(!disabled(&harness, &t(key)), "{key}");
        button(&harness, &t(key)).click();
        harness.run_steps(3);
    }
    assert_eq!(
        harness.state().1,
        vec![
            IndexesAction::SetHidden("a_1".into(), true),
            IndexesAction::EditCopy("a_1".into()),
            IndexesAction::DropRequested("a_1".into()),
        ]
    );
    // Not a TTL index: nothing to change.
    assert_eq!(
        harness.query_all_by_label(&t("indexes.action_ttl")).count(),
        0
    );

    // A hidden index offers Unhide instead of Hide.
    let hidden = IndexSpec {
        options: IndexOptions {
            hidden: true,
            ..IndexOptions::default()
        },
        ..plain("a_1")
    };
    let state = IndexesState {
        indexes: vec![listed(hidden)],
        ..tab("a_1", None)
    };
    let mut harness = open_tab(state, false);
    assert_eq!(
        harness
            .query_all_by_label(&t("indexes.action_hide"))
            .count(),
        0
    );
    button(&harness, &t("indexes.action_unhide")).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().1,
        vec![IndexesAction::SetHidden("a_1".into(), false)]
    );

    // Only an index that expires offers Change TTL.
    let mut harness = open_tab(tab("ttl_1", None), false);
    button(&harness, &t("indexes.action_ttl")).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().1,
        vec![IndexesAction::ChangeTtl("ttl_1".into())]
    );
}

#[test]
fn a_read_only_connection_disables_every_write_control_and_shows_why() {
    let harness = open_tab(tab("ttl_1", None), true);
    for key in WRITES
        .into_iter()
        .chain(["indexes.action_ttl", "indexes.create"])
    {
        assert!(disabled(&harness, &t(key)), "{key}");
    }
    // Copy as JSON is a read.
    assert!(!disabled(&harness, &t("indexes.copy_json")));
    harness.get_by_label(&t("connection.read_only_reason"));

    // The modals: their write buttons are off and say why, whatever else is true.
    let badge = ConnectionBadge::default().with_read_only(true);
    let drop = DropConfirm::new("a_1".into()).with_typed("a_1".into());
    let harness = open_dialog(Dialog::Drop(drop), badge, false);
    assert!(dialog_disabled(&harness, &t("indexes.drop.confirm")));
    harness.get_by_label(&t("connection.read_only_reason"));

    let old = DropOld {
        old: "a_1".into(),
        new: "a_1_v2".into(),
    };
    let harness = open_dialog(Dialog::Old(old), badge, false);
    assert!(dialog_disabled(&harness, &t("indexes.drop_old.drop")));
    harness.get_by_label(&t("connection.read_only_reason"));

    let prompt = TtlPrompt::new("ttl_1".into(), Some(60));
    let harness = open_dialog(Dialog::Ttl(prompt), badge, false);
    assert!(dialog_disabled(&harness, &t("indexes.ttl.apply")));
    harness.get_by_label(&t("connection.read_only_reason"));
}

#[test]
fn write_controls_wait_while_a_write_is_in_flight() {
    let sent = Some(IndexWrite::Hide { name: "a_1".into() });
    let harness = open_tab(tab("ttl_1", sent), false);
    for key in WRITES.into_iter().chain(["indexes.action_ttl"]) {
        assert!(disabled(&harness, &t(key)), "{key}");
    }
    harness.get_by_label(&t("indexes.write_busy"));
}

#[test]
fn id_index_cannot_be_dropped_or_hidden() {
    let mut harness = open_tab(tab("_id_", None), false);
    for key in WRITES {
        assert!(disabled(&harness, &t(key)), "{key} is not offered for _id_");
        // Clicking a disabled button reports nothing.
        button(&harness, &t(key)).click();
        harness.run_steps(2);
    }
    assert!(harness.state().1.is_empty());
    harness.get_by_label(&t("indexes.id_locked"));
    // Other indexes of the same list stay writable.
    let other = open_tab(tab("a_1", None), false);
    for key in WRITES {
        assert!(!disabled(&other, &t(key)), "{key}");
    }
    assert_eq!(other.query_all_by_label(&t("indexes.id_locked")).count(), 0);
}

#[test]
fn copy_as_json_puts_the_definition_on_the_clipboard_action_from_the_button() {
    let geo = IndexSpec {
        raw: doc! { "key": { "loc": "2d" }, "name": "loc_2d", "bits": 26, "v": 2 },
        ..mock_index_spec_named("loc_2d", &[("loc", KeyKind::Geo2d)])
    };
    let state = IndexesState {
        indexes: vec![listed(geo.clone())],
        ..tab("loc_2d", None)
    };
    let mut harness = open_tab(state, true);
    button(&harness, &t("indexes.copy_json")).click();
    harness.run_steps(3);
    let seen = harness.state().1.clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].clipboard_text(), Some(geo.to_json().as_str()));
}

/// One of the modals under test.
#[derive(Clone)]
enum Dialog {
    Drop(DropConfirm),
    Old(DropOld),
    Ttl(TtlPrompt),
}

struct DialogFrame {
    dialog: Dialog,
    badge: ConnectionBadge,
    busy: bool,
    /// The modal is on screen; a closed modal is simply not drawn, as in the app.
    shown: bool,
    seen: Vec<IndexesAction>,
}

fn open_dialog(
    dialog: Dialog,
    badge: ConnectionBadge,
    busy: bool,
) -> Harness<'static, DialogFrame> {
    let frame = DialogFrame {
        dialog,
        badge,
        busy,
        shown: true,
        seen: Vec::new(),
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(
            |ctx, frame: &mut DialogFrame| {
                if !frame.shown {
                    return;
                }
                let context = DialogContext {
                    connection: "local",
                    badge: &frame.badge,
                    collection: "shop.orders",
                    busy: frame.busy,
                };
                let i18n = I18n::english();
                let action = match &frame.dialog {
                    Dialog::Drop(c) => show_drop_confirm(ctx, c, &context, &i18n),
                    Dialog::Old(o) => show_drop_old(ctx, o, &context, &i18n),
                    Dialog::Ttl(p) => show_ttl_prompt(ctx, p, &context, &i18n),
                };
                // Typing is applied the way the reducer would, to follow it over several frames.
                match (&action, &frame.dialog) {
                    (Some(IndexesAction::DropTyped(text)), Dialog::Drop(c)) => {
                        frame.dialog = Dialog::Drop(c.clone().with_typed(text.clone()));
                    }
                    (Some(IndexesAction::TtlTyped(text)), Dialog::Ttl(p)) => {
                        frame.dialog = Dialog::Ttl(p.clone().with_text(text.clone()));
                    }
                    _ => {}
                }
                frame.seen.extend(action);
            },
            frame,
        );
    harness.run_steps(10);
    harness
}

fn dialog_button<'a>(
    harness: &'a Harness<'static, DialogFrame>,
    label: &'a str,
) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::Button, label)
}

fn dialog_disabled(harness: &Harness<'static, DialogFrame>, label: &str) -> bool {
    dialog_button(harness, label).accesskit_node().is_disabled()
}

fn production() -> ConnectionBadge {
    ConnectionBadge::new(None, true)
}

#[test]
fn dropping_requires_typing_the_exact_index_name_in_the_modal() {
    let name = "My.Idx é";
    let drop = t("indexes.drop.confirm");
    let caption = I18n::english().t_with("indexes.drop.type_name", &[("name", name)]);
    let mut harness = open_dialog(
        Dialog::Drop(DropConfirm::new(name.into())),
        ConnectionBadge::default(),
        false,
    );
    assert!(
        dialog_button(&harness, &drop)
            .accesskit_node()
            .is_disabled()
    );
    // The text box has the focus when the modal opens; a different case does not confirm.
    harness
        .get_by_role_and_label(Role::TextInput, &caption)
        .type_text("my.idx é");
    harness.run_steps(4);
    assert_eq!(
        harness.state().seen.last(),
        Some(&IndexesAction::DropTyped("my.idx é".into())),
        "reported as typed"
    );
    assert!(
        dialog_button(&harness, &drop)
            .accesskit_node()
            .is_disabled()
    );

    // The exact name enables it, and clicking reports the confirmation once.
    let exact = DropConfirm::new(name.into()).with_typed(name.into());
    let mut harness = open_dialog(Dialog::Drop(exact), ConnectionBadge::default(), false);
    assert!(
        !dialog_button(&harness, &drop)
            .accesskit_node()
            .is_disabled()
    );
    dialog_button(&harness, &drop).click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![IndexesAction::DropConfirmed]);
}

#[test]
fn the_drop_modal_shows_the_production_notice_and_waits_while_busy() {
    let name = "a_1";
    let confirm = DropConfirm::new(name.into()).with_typed(name.into());
    let harness = open_dialog(Dialog::Drop(confirm.clone()), production(), false);
    harness.get_by_label(&t("write_warning.production_line"));
    let calm = open_dialog(
        Dialog::Drop(confirm.clone()),
        ConnectionBadge::default(),
        false,
    );
    assert_eq!(
        calm.query_all_by_label(&t("write_warning.production_line"))
            .count(),
        0
    );
    let busy = open_dialog(Dialog::Drop(confirm), ConnectionBadge::default(), true);
    assert!(
        dialog_button(&busy, &t("indexes.drop.confirm"))
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        dialog_button(&busy, &t("common.cancel"))
            .accesskit_node()
            .is_disabled()
    );
    busy.get_by_label(&t("indexes.builder.busy"));
}

#[test]
fn escape_cancels_each_modal_unless_it_is_busy() {
    let cases = [
        (
            Dialog::Drop(DropConfirm::new("a_1".into())),
            IndexesAction::DropCancelled,
        ),
        (
            Dialog::Old(DropOld {
                old: "a_1".into(),
                new: "a_1_v2".into(),
            }),
            IndexesAction::KeepOld,
        ),
        (
            Dialog::Ttl(TtlPrompt::new("ttl_1".into(), Some(60))),
            IndexesAction::TtlCancelled,
        ),
    ];
    for (dialog, expected) in cases {
        let mut harness = open_dialog(dialog.clone(), ConnectionBadge::default(), false);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
        assert_eq!(harness.state().seen, vec![expected.clone()], "{expected:?}");
        let mut busy = open_dialog(dialog, ConnectionBadge::default(), true);
        busy.key_press(egui::Key::Escape);
        busy.run_steps(3);
        assert!(busy.state().seen.is_empty(), "{expected:?} while busy");
    }
}

#[test]
fn the_ask_drop_old_modal_defaults_to_keeping_both() {
    let old = DropOld {
        old: "email_1".into(),
        new: "email_1_v2".into(),
    };
    let mut harness = open_dialog(Dialog::Old(old), ConnectionBadge::default(), false);
    let message = I18n::english().t_with(
        "indexes.drop_old.message",
        &[("new", "email_1_v2"), ("old", "email_1")],
    );
    harness.get_by_label(&message);
    // A stray Enter keeps both: Keep is the one focused.
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![IndexesAction::KeepOld]);

    let old = DropOld {
        old: "email_1".into(),
        new: "email_1_v2".into(),
    };
    let mut harness = open_dialog(Dialog::Old(old), ConnectionBadge::default(), false);
    dialog_button(&harness, &t("indexes.drop_old.drop")).click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![IndexesAction::ConfirmDropOld]);
}

#[test]
fn the_ttl_prompt_applies_only_a_positive_number_of_seconds() {
    let apply = t("indexes.ttl.apply");
    let ok = open_dialog(
        Dialog::Ttl(TtlPrompt::new("ttl_1".into(), Some(60))),
        ConnectionBadge::default(),
        false,
    );
    assert!(!dialog_button(&ok, &apply).accesskit_node().is_disabled());
    let mut ok = ok;
    dialog_button(&ok, &apply).click();
    ok.run_steps(3);
    assert_eq!(
        ok.state().seen,
        vec![IndexesAction::SetTtl("ttl_1".into(), 60)]
    );
    for bad in ["", "-4", "soon"] {
        let prompt = TtlPrompt::new("ttl_1".into(), Some(60)).with_text(bad.into());
        let harness = open_dialog(Dialog::Ttl(prompt), ConnectionBadge::default(), false);
        assert!(
            dialog_button(&harness, &apply)
                .accesskit_node()
                .is_disabled(),
            "{bad:?}"
        );
        harness.get_by_label(&t("indexes.builder.problem_ttl_non_negative"));
    }
}

#[test]
fn index_action_controls_are_labelled_and_keyboard_reachable() {
    // The per-index buttons.
    let mut harness = open_tab(tab("ttl_1", None), false);
    let expected = [
        t("indexes.copy_json"),
        t("indexes.action_hide"),
        t("indexes.action_edit_copy"),
        t("indexes.action_ttl"),
        t("indexes.action_drop"),
    ];
    let mut reached = Vec::new();
    for _ in 0..30 {
        harness.key_press(egui::Key::Tab);
        harness.run();
        reached.extend(
            harness
                .root()
                .children_recursive()
                .find(|n| n.accesskit_node().is_focused())
                .and_then(|n| n.accesskit_node().label()),
        );
    }
    for label in &expected {
        assert!(reached.contains(label), "{label} not reached: {reached:?}");
        let node = harness.get_by_role_and_label(Role::Button, label);
        assert!(node.accesskit_node().label().is_some_and(|l| !l.is_empty()));
    }

    // The drop confirmation: the text box is named by its caption, both buttons are named.
    let drop = DropConfirm::new("ttl_1".into()).with_typed("ttl_1".into());
    let mut harness = open_dialog(Dialog::Drop(drop), ConnectionBadge::default(), false);
    let caption = I18n::english().t_with("indexes.drop.type_name", &[("name", "ttl_1")]);
    harness.get_by_role_and_label(Role::TextInput, &caption);
    reach_all(
        &mut harness,
        &[t("common.cancel"), t("indexes.drop.confirm"), caption],
    );
    harness.get_by_label(&I18n::english().t_with(
        "indexes.drop.message",
        &[
            ("name", "ttl_1"),
            ("collection", "shop.orders"),
            ("connection", "local"),
        ],
    ));

    // The question after a copy was built.
    let old = DropOld {
        old: "email_1".into(),
        new: "email_1_v2".into(),
    };
    let mut harness = open_dialog(Dialog::Old(old), ConnectionBadge::default(), false);
    reach_all(
        &mut harness,
        &[t("indexes.drop_old.keep"), t("indexes.drop_old.drop")],
    );
    harness.get_by_label(&t("indexes.drop_old.title"));

    // The TTL input.
    let prompt = TtlPrompt::new("ttl_1".into(), Some(60));
    let mut harness = open_dialog(Dialog::Ttl(prompt), ConnectionBadge::default(), false);
    reach_all(
        &mut harness,
        &[
            t("indexes.builder.ttl"),
            t("indexes.ttl.apply"),
            t("common.cancel"),
        ],
    );
}

/// Tab reaches a control named (or captioned) by each of `labels`.
fn reach_all(harness: &mut Harness<'static, DialogFrame>, labels: &[String]) {
    let mut reached = Vec::new();
    for _ in 0..12 {
        harness.key_press(egui::Key::Tab);
        harness.run();
        let focused = harness
            .root()
            .children_recursive()
            .find(|n| n.accesskit_node().is_focused());
        reached.extend(focused.and_then(|n| {
            n.accesskit_node().label().or_else(|| {
                n.accesskit_node()
                    .labelled_by()
                    .next()
                    .and_then(|caption| caption.label())
            })
        }));
    }
    for label in labels {
        assert!(reached.contains(label), "{label} not reached: {reached:?}");
    }
    // Everything on screen that is a control has a name.
    let unnamed = names(harness)
        .iter()
        .filter(|n| n.trim().is_empty())
        .count();
    assert_eq!(unnamed, 0);
}

#[test]
fn a_copy_builder_names_its_original_and_keeps_hashed_and_2d_rows_read_only() {
    use curlybrace_ui::viewmodel::IndexBuilderState;
    use curlybrace_ui::views::index_builder::{BuilderContext, show_index_builder};
    let original = mock_index_spec_named(
        "loc_2d_kind_1",
        &[("loc", KeyKind::Geo2d), ("kind", KeyKind::Asc)],
    );
    let builder = IndexBuilderState::from_copy(
        Namespace::new("shop", "orders"),
        &original,
        "loc_2d_kind_1_v2".into(),
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 900.0))
        .build_state(
            |ctx, builder: &mut IndexBuilderState| {
                let context = BuilderContext {
                    connection: "local",
                    read_only: false,
                    busy: false,
                    existing: &[],
                };
                show_index_builder(ctx, builder, &context, &I18n::english());
            },
            builder,
        );
    harness.run_steps(10);
    let i18n = I18n::english();
    harness.get_by_label(&i18n.t_with("indexes.builder.title_copy", &[("name", "loc_2d_kind_1")]));
    harness
        .get_by_label(&i18n.t_with("indexes.builder.editing_note", &[("name", "loc_2d_kind_1")]));
    harness.get_by_label(&t("indexes.builder.fixed_keys_note"));
    let kind = |n: &str| i18n.t_with("indexes.builder.key_type_n", &[("n", n)]);
    let field = |n: &str| i18n.t_with("indexes.builder.key_field_n", &[("n", n)]);
    // The 2d row shows its type and cannot be changed; the ordinary row can.
    let first = kind("1");
    let fixed = harness.get_by_role_and_label(Role::ComboBox, &first);
    assert!(fixed.accesskit_node().is_disabled());
    assert_eq!(
        fixed.accesskit_node().value().as_deref(),
        Some(t("indexes.builder.kind_geo_flat").as_str())
    );
    assert!(
        harness
            .get_by_role_and_label(Role::TextInput, &field("1"))
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        !harness
            .get_by_role_and_label(Role::ComboBox, &kind("2"))
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        !harness
            .get_by_role_and_label(Role::TextInput, &field("2"))
            .accesskit_node()
            .is_disabled()
    );
}

fn focused_label(harness: &Harness<'static, DialogFrame>) -> Option<String> {
    harness
        .root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
        .map(|s| s.to_string())
}

#[test]
fn a_modal_takes_the_focus_every_time_it_opens() {
    let old = || DropOld {
        old: "email_1".into(),
        new: "email_1_v2".into(),
    };
    let keep = t("indexes.drop_old.keep");
    let mut harness = open_dialog(Dialog::Old(old()), ConnectionBadge::default(), false);
    assert_eq!(focused_label(&harness), Some(keep.clone()), "first open");

    harness.state_mut().shown = false;
    harness.run_steps(4);
    harness.state_mut().dialog = Dialog::Old(old());
    harness.state_mut().shown = true;
    harness.run_steps(10);
    assert_eq!(focused_label(&harness), Some(keep), "reopened");

    let caption = t("indexes.drop.type_name");
    let drop = || Dialog::Drop(DropConfirm::new("a_1".into()));
    let mut harness = open_dialog(drop(), ConnectionBadge::default(), false);
    assert!(focused_label(&harness).is_some(), "first open");
    harness.state_mut().shown = false;
    harness.run_steps(4);
    harness.state_mut().shown = true;
    harness.run_steps(10);
    let focused = harness
        .root()
        .children_recursive()
        .any(|n| n.accesskit_node().is_focused() && n.accesskit_node().role() == Role::TextInput);
    assert!(focused, "reopened, the name box is focused ({caption})");
}
