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
use support::compare_apply_states::{reviewing, with_replace};

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

    // The compare pickers: the closed combo's text is not in egui's accesskit tree, so open the
    // source picker and match the exact badged entry (the explanatory note also says
    // "Read-only", so a contains-match would be vacuous).
    let mut compare = pickers();
    open_combo(&mut compare, "compare.session.source_connection");
    let labelled = I18n::english().t_with(
        "connection.name_with_read_only",
        &[("name", "Vault"), ("badge", &badge)],
    );
    compare.get_by_role_and_label(Role::Button, &labelled);
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

#[test]
fn a_long_read_only_name_never_pushes_the_badge_out_of_its_cell() {
    let long = "a-very-long-connection-name-that-is-far-wider-than-the-name-column-".repeat(3);
    let mut cfg = read_only(&long);
    cfg.uri = "mongodb://neighbour.example.com".into();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(400.0, 500.0))
        .build_ui_state(
            |ui, state: &mut ConnectionsState| {
                show_connections(ui, state, &I18n::english());
            },
            ConnectionsState::new(vec![cfg]),
        );
    harness.run_steps(3);
    let badge = t("connection.read_only_badge");
    let badge_box = harness
        .get_by_label(&badge)
        .accesskit_node()
        .bounding_box()
        .expect("badge has bounds");
    let server_box = harness
        .get_by_label("neighbour.example.com")
        .accesskit_node()
        .bounding_box()
        .expect("server cell has bounds");
    assert!(
        badge_box.x1 <= server_box.x0,
        "badge {badge_box:?} must stay inside the name cell, left of the server column {server_box:?}"
    );
}

#[test]
fn review_modal_apply_is_disabled_with_the_reason_on_a_read_only_target() {
    let state = reviewing(with_replace());
    let mut compare_state = state.compare;
    compare_state.connections_read_only = (false, true);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(
            |ui, (s, c): &mut (CompareState, ConnectionsState)| {
                show_compare(ui, s, c, &I18n::english());
            },
            (compare_state, state.conn),
        );
    harness.run_steps(5);
    let apply_label = t("compare.apply.review.apply");
    let apply = harness.get_by_role_and_label(Role::Button, &apply_label);
    assert!(apply.accesskit_node().is_disabled(), "Apply is disabled");
    // The pending panel and the modal both state the reason.
    let reason = t("compare.apply.reason.read_only_target");
    assert!(harness.query_all_by_label(&reason).count() >= 1);
}

#[test]
fn row_menu_entries_say_why_they_are_disabled_on_hover() {
    let mut docs = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            |ui, state: &mut TreeState| {
                let target = DocTarget {
                    connection: "Vault",
                    production: false,
                    read_only: true,
                };
                let page = Page {
                    docs: vec![doc! { "_id": 1_i32, "name": "x" }],
                    skip: 0,
                    has_more: false,
                };
                show_documents(ui, ViewMode::Tree, &page, state, target, &I18n::english());
            },
            TreeState::default(),
        );
    docs.run_steps(3);
    docs.get_by_label(&I18n::english().t_with("doc.header", &[("n", "1")]))
        .click_secondary();
    docs.run_steps(3);
    let reason = t("connection.read_only_reason");
    for key in ["doc.edit", "doc.clone", "doc.delete"] {
        let label = t(key);
        docs.get_by_role_and_label(Role::Button, &label).hover();
        docs.run_steps(8);
        // The page-level note shows the same text, so the hover tooltip makes it appear twice.
        assert!(
            docs.query_all_by_label(&reason).count() >= 2,
            "{key} shows the reason on hover"
        );
    }
}
