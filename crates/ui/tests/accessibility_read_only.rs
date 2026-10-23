//! Read-only connections in the accesskit tree: the marker is text, write actions are disabled
//! with a stated reason (not just refused after a click), and the compare target picker leaves
//! read-only connections out while the source picker keeps them. Fast, offline, no GPU.
mod support;

use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::store::Page;
use curlybrace_ui::I18n;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::viewmodel::compare::CompareState;
use curlybrace_ui::viewmodel::{ConnectionBadge, TreeState, ViewMode};
use curlybrace_ui::views::compare::show_compare;
use curlybrace_ui::views::connections::{ConnectionsState, show_connections};
use curlybrace_ui::views::documents::{DocTarget, show_connection_header, show_documents};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::compare_apply_states::with_replace;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn read_only(name: &str) -> ConnectionConfig {
    let mut cfg = ConnectionConfig::new(name, "mongodb://locked.example.com");
    cfg.read_only = true;
    cfg
}

fn writable(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, "mongodb://open.example.com")
}

#[test]
fn read_only_badge_is_text_on_list_header_and_compare() {
    let badge = t("connection.read_only_badge");

    // The connection list row.
    let mut list = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, state: &mut ConnectionsState| {
                show_connections(ui, state, &I18n::english());
            },
            ConnectionsState::new(vec![read_only("Vault"), writable("Scratch")]),
        );
    list.run();
    assert_eq!(list.query_all_by_label(&badge).count(), 1, "one row only");

    // The session header.
    let mut header = Harness::new_ui(|ui| {
        let badge = ConnectionBadge::new(None, false).with_read_only(true);
        show_connection_header(ui, "Vault", &badge, &I18n::english());
    });
    header.run();
    header.get_by_label(&badge);

    // The compare pickers (source is picked first, so its closed combo names the badge).
    let connections = ConnectionsState::new(vec![read_only("Vault"), writable("Scratch")]);
    let mut compare = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, (s, c): &mut (CompareState, ConnectionsState)| {
                show_compare(ui, s, c, &I18n::english());
            },
            (CompareState::default(), connections),
        );
    compare.run_steps(3);
    assert!(
        compare.query_all_by_label_contains(&badge).count() >= 1,
        "the compare picker names the badge as text"
    );
}

#[test]
fn write_actions_are_disabled_with_a_stated_reason() {
    let reason = t("connection.read_only_reason");
    let mut docs = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            |ui, state: &mut TreeState| {
                let target = DocTarget {
                    connection: "Vault",
                    production: false,
                    read_only: true,
                };
                show_documents(
                    ui,
                    ViewMode::Tree,
                    &Page {
                        docs: vec![doc! { "_id": 1_i32, "name": "x" }],
                        skip: 0,
                        has_more: false,
                    },
                    state,
                    target,
                    &I18n::english(),
                );
            },
            TreeState::default(),
        );
    docs.run_steps(3);
    let insert_label = t("doc.insert");
    let insert = docs.get_by_role_and_label(Role::Button, &insert_label);
    assert!(insert.accesskit_node().is_disabled(), "insert is disabled");
    docs.get_by_label(&reason);

    // Row actions in the context menu are listed but disabled.
    docs.get_by_label(&I18n::english().t_with("doc.header", &[("n", "1")]))
        .click_secondary();
    docs.run_steps(3);
    for key in ["doc.edit", "doc.clone", "doc.delete"] {
        let label = t(key);
        let entry = docs.get_by_role_and_label(Role::Button, &label);
        assert!(entry.accesskit_node().is_disabled(), "{key} is disabled");
    }

    // The compare review entry is disabled and says why.
    let mut state = with_replace();
    state.compare.connections_read_only = (false, true);
    let mut compare = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(
            |ui, (s, c): &mut (CompareState, ConnectionsState)| {
                show_compare(ui, s, c, &I18n::english());
            },
            (state.compare, state.conn),
        );
    compare.run_steps(5);
    let review_label = t("compare.apply.pending.review");
    let review = compare.get_by_role_and_label(Role::Button, &review_label);
    assert!(review.accesskit_node().is_disabled(), "review is disabled");
    compare.get_by_label(&t("compare.apply.reason.read_only_target"));
}

type Picked = (CompareState, ConnectionsState, Option<CompareAction>);

fn pickers() -> Harness<'static, Picked> {
    let connections = ConnectionsState::new(vec![
        writable("Scratch"),
        read_only("Vault"),
        writable("Staging"),
    ]);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_ui_state(
            |ui, (s, c, out): &mut Picked| {
                *out = show_compare(ui, s, c, &I18n::english()).or(out.take());
            },
            (CompareState::default(), connections, None),
        );
    harness.run_steps(3);
    harness
}

fn open_combo(harness: &mut Harness<'static, Picked>, label_key: &str) {
    harness
        .get_by_role_and_label(Role::ComboBox, &t(label_key))
        .click();
    harness.run_steps(3);
}

#[test]
fn compare_target_picker_excludes_read_only_and_says_why() {
    let mut harness = pickers();
    open_combo(&mut harness, "compare.session.target_connection");
    let vault = I18n::english().t_with(
        "connection.name_with_read_only",
        &[
            ("name", "Vault"),
            ("badge", &t("connection.read_only_badge")),
        ],
    );
    let entry = harness.get_by_label(&vault);
    assert!(
        entry.accesskit_node().is_disabled(),
        "Vault cannot be picked"
    );
    harness.get_by_label("Staging");
    harness.get_by_label(&t("compare.session.target_read_only"));
}

#[test]
fn compare_source_picker_still_lists_read_only() {
    let mut harness = pickers();
    open_combo(&mut harness, "compare.session.source_connection");
    let vault = I18n::english().t_with(
        "connection.name_with_read_only",
        &[
            ("name", "Vault"),
            ("badge", &t("connection.read_only_badge")),
        ],
    );
    let entry = harness.get_by_label(&vault);
    assert!(!entry.accesskit_node().is_disabled(), "Vault is pickable");
}
