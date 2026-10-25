use super::*;
use crate::app::session::SessionView;
use crate::viewmodel::connection_tag::PRODUCTION_BADGE_KEY;
use crate::viewmodel::{
    CopyClipboard, NodeRef, PasteTarget, SelectMode, SidebarSelection, SidebarState,
};
use crate::views::sidebar::db_row_label;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::Namespace;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use uuid::Uuid;

/// `egui::SidePanel`'s default width.
const DEFAULT_SIDEBAR_WIDTH: f32 = 200.0;

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn tree(db: &str, collection: &str) -> SidebarState {
    SidebarState::default()
        .with_databases(names(&[db]))
        .toggled(db)
        .with_collections(db, names(&[collection]))
}

struct Fixture {
    state: AppState,
    active: Uuid,
    parked: Uuid,
    connecting: Uuid,
    failed: Uuid,
}

/// Four open roots: `Alpha` active and connected, `Beta` parked and connected, `Gamma`
/// connecting, `Delta` failed.
fn fixture() -> Fixture {
    let configs: Vec<_> = ["Alpha", "Beta", "Gamma", "Delta"]
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids: Vec<Uuid> = configs.iter().map(|c| c.id).collect();
    let mut state = AppState::new(configs);
    state.open = ids.clone();
    state.active = Some(ids[0]);
    state.status = ConnStatus::Connected;
    state.topology = Some(Topology::ReplicaSet);
    state.sidebar = tree("shop", "orders");
    let view = |status, failure, sidebar| SessionView {
        status,
        failure,
        sidebar,
        ..SessionView::default()
    };
    state.parked.insert(
        ids[1],
        view(ConnStatus::Connected, None, tree("crm", "users")),
    );
    state.parked.insert(
        ids[2],
        view(ConnStatus::Connecting, None, SidebarState::default()),
    );
    state.parked.insert(
        ids[3],
        view(
            ConnStatus::Failed,
            Some(CoreError::Database("refused".into())),
            SidebarState::default(),
        ),
    );
    Fixture {
        state,
        active: ids[0],
        parked: ids[1],
        connecting: ids[2],
        failed: ids[3],
    }
}

fn harness(state: AppState) -> Harness<'static, (AppState, Option<Input>)> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 900.0))
        .build_ui_state(
            |ui, (state, out): &mut (AppState, Option<Input>)| {
                *out = show_roots(ui, state, &I18n::english()).or(out.take());
            },
            (state, None),
        );
    harness.run();
    harness
}

/// The roots in the real resizable sidebar panel at its default width. Returns the harness
/// and the panel's final width (a header that needs more than the panel offers grows it).
fn panel_harness(state: AppState) -> Harness<'static, f32> {
    let shown = state;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 900.0))
        .build_state(
            move |ctx, width: &mut f32| {
                let panel = egui::SidePanel::left("sidebar")
                    .resizable(true)
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| show_roots(ui, &shown, &I18n::english()));
                    });
                *width = panel.response.rect.width();
            },
            0.0,
        );
    harness.run();
    harness
}

fn click(state: AppState, label: &str) -> Option<Input> {
    let mut harness = harness(state);
    harness.get_by_label(label).click();
    harness.run();
    harness.state_mut().1.take()
}

fn frame(state: &AppState) -> Option<Input> {
    let ctx = egui::Context::default();
    let i18n = I18n::english();
    let mut out = None;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| out = show_roots(ui, state, &i18n));
    });
    out
}

#[test]
fn renders_zero_one_and_several_roots_without_input() {
    let all = fixture().state;
    assert!(frame(&AppState::default()).is_none());
    let mut one = AppState::new(all.conn.connections.clone());
    one.open = vec![all.open[0]];
    one.active = Some(all.open[0]);
    one.status = ConnStatus::Connected;
    assert!(frame(&one).is_none());
    assert!(frame(&all).is_none());
}

#[test]
fn opening_a_collection_under_a_parked_root_is_tagged_with_that_session() {
    let f = fixture();
    match click(f.state, "users") {
        Some(Input::Sidebar(SidebarAction::InSession { session, action })) => {
            assert_eq!(session, f.parked);
            assert!(matches!(*action, SidebarAction::Open(ns) if ns.collection == "users"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn the_active_roots_actions_are_tagged_too() {
    let f = fixture();
    match click(f.state, "orders") {
        Some(Input::Sidebar(SidebarAction::InSession { session, .. })) => {
            assert_eq!(session, f.active);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn disconnect_closes_that_root() {
    let f = fixture();
    let label = I18n::english().t_with("sidebar.disconnect_named", &[("name", "Gamma")]);
    let got = click(f.state, &label);
    assert!(matches!(got, Some(Input::Disconnect(id)) if id == f.connecting));
}

#[test]
fn reconnect_is_offered_on_a_failed_root_and_connects_it() {
    let f = fixture();
    let label = I18n::english().t_with("sidebar.reconnect_named", &[("name", "Delta")]);
    let got = click(f.state, &label);
    assert!(matches!(got, Some(Input::Connect(id)) if id == f.failed));
}

#[test]
fn only_a_failed_root_offers_reconnect() {
    let f = fixture();
    let harness = harness(f.state);
    let prefix = I18n::english().t("sidebar.reconnect");
    let offered = harness.get_all_by_label_contains(&prefix).count();
    assert_eq!(offered, 1);
}

#[test]
fn clicking_a_root_name_activates_it() {
    let f = fixture();
    let got = click(f.state, "Gamma");
    assert!(matches!(got, Some(Input::Activate(id)) if id == f.connecting));
}

#[test]
fn the_connections_button_asks_for_the_manager() {
    let got = click(fixture().state, &I18n::english().t("sidebar.connections"));
    assert!(matches!(got, Some(Input::ShowConnections)));
}

#[test]
fn a_failed_root_shows_its_error_and_a_connecting_root_only_its_status() {
    let f = fixture();
    let i18n = I18n::english();
    let harness = harness(f.state);
    harness.get_by_label(&i18n.t("sidebar.root_failed"));
    harness.get_by_label(&i18n.error(&CoreError::Database("refused".into())));
    harness.get_by_label(&i18n.t("sidebar.status_connecting"));
}

#[test]
fn the_header_shows_name_status_and_topology_line_and_the_buttons() {
    let f = fixture();
    let i18n = I18n::english();
    let harness = harness(f.state);
    harness.get_by_label("Alpha");
    let line = i18n.t_with(
        "sidebar.status_topology",
        &[
            ("status", &i18n.t("sidebar.status_connected")),
            ("topology", &i18n.t("sidebar.topology_replica_set")),
        ],
    );
    harness.get_by_label(&line);
    harness.get_by_label(&i18n.t_with("sidebar.disconnect_named", &[("name", "Alpha")]));
    harness.get_by_label(&i18n.t_with("sidebar.toggle_named", &[("name", "Alpha")]));
}

#[test]
fn no_database_heading_repeats_under_a_root() {
    let harness = harness(fixture().state);
    let title = I18n::english().t("sidebar.title");
    assert_eq!(harness.query_all_by_label(&title).count(), 0);
}

#[test]
fn a_roots_refresh_is_tagged_with_its_session() {
    let f = fixture();
    let label = I18n::english().t("sidebar.refresh");
    let mut harness = harness(f.state);
    harness.get_all_by_label(&label).next().unwrap().click();
    harness.run();
    match harness.state_mut().1.take() {
        Some(Input::Sidebar(SidebarAction::InSession { session, action })) => {
            assert_eq!(session, f.active);
            assert!(matches!(*action, SidebarAction::Refresh));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_failed_root_keeps_the_sidebar_at_its_default_width() {
    let i18n = I18n::english();
    let delta = [("name", "Delta")];
    let harness = panel_harness(fixture().state);
    let panel_width = *harness.state();
    assert!(
        (panel_width - DEFAULT_SIDEBAR_WIDTH).abs() < 0.5,
        "the panel grew to {panel_width}"
    );
    let toggle = harness
        .get_by_label(&i18n.t_with("sidebar.toggle_named", &delta))
        .rect();
    let disconnect = harness
        .get_by_label(&i18n.t_with("sidebar.disconnect_named", &delta))
        .rect();
    let reconnect = harness
        .get_by_label(&i18n.t_with("sidebar.reconnect_named", &delta))
        .rect();
    assert!(disconnect.right() <= panel_width, "{:?}", disconnect);
    assert!(reconnect.right() <= panel_width, "{:?}", reconnect);
    // Line 1 holds the toggle and Disconnect; Reconnect sits below them with the status.
    assert!((toggle.center().y - disconnect.center().y).abs() < 1.0);
    assert!(reconnect.top() >= disconnect.bottom());
}

#[test]
fn a_tagged_root_keeps_the_sidebar_at_its_default_width() {
    use curlybrace_core::connection::ConnectionColor;
    let long = "A very long production connection name for the sidebar";
    let cases = [
        ("colour", "Alpha", Some(ConnectionColor::Red), false),
        ("production", "Alpha", None, true),
        ("both", "Alpha", Some(ConnectionColor::Purple), true),
        ("both, long name", long, Some(ConnectionColor::Purple), true),
    ];
    for (case, name, color, production) in cases {
        let mut f = fixture();
        let config = &mut f.state.conn.connections[0];
        config.name = name.into();
        config.color = color.map(curlybrace_core::connection::SavedColor::Known);
        config.production = production;
        f.state.production = production;
        let harness = panel_harness(f.state);
        let panel_width = *harness.state();
        assert!(
            (panel_width - DEFAULT_SIDEBAR_WIDTH).abs() < 0.5,
            "{case}: the panel grew to {panel_width}"
        );
        let label = I18n::english().t_with("sidebar.disconnect_named", &[("name", name)]);
        let disconnect = harness.get_by_label(&label).rect();
        assert!(disconnect.right() <= panel_width, "{case}: {disconnect:?}");
        // The tag stays visible on the root: the colour's name labels its glyph.
        if let Some(color) = color {
            let tag = crate::viewmodel::color_tag(color);
            harness.get_by_label(&I18n::english().t(tag.name_key));
        }
        if production {
            harness.get_by_label(&I18n::english().t(PRODUCTION_BADGE_KEY));
        }
    }
}

#[test]
fn the_name_is_centred_on_the_line_of_the_toggle_and_buttons() {
    let i18n = I18n::english();
    let harness = panel_harness(fixture().state);
    let toggle = harness
        .get_by_label(&i18n.t_with("sidebar.toggle_named", &[("name", "Alpha")]))
        .rect();
    let name = harness.get_by_label("Alpha").rect();
    let gap = (toggle.center().y - name.center().y).abs();
    assert!(gap < 1.0, "the name is {gap} px off the line centre");
}

#[test]
fn a_very_long_name_wraps_and_keeps_the_buttons_on_screen() {
    let mut f = fixture();
    let long = "A very long connection name that cannot fit one line of the sidebar";
    f.state.conn.connections[0].name = long.into();
    let harness = harness(f.state);
    harness.get_by_label(long);
    let label = I18n::english().t_with("sidebar.disconnect_named", &[("name", long)]);
    let rect = harness.get_by_label(&label).rect();
    assert!(rect.right() <= 320.0, "{rect:?}");
}

fn selection_action(input: Option<Input>) -> (Uuid, SidebarAction) {
    match input {
        Some(Input::Sidebar(SidebarAction::InSession { session, action })) => (session, *action),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_command_click_toggles_and_a_shift_click_extends_without_opening() {
    for (modifiers, expected) in [
        (egui::Modifiers::COMMAND, SelectMode::Toggle),
        (egui::Modifiers::SHIFT, SelectMode::Extend),
    ] {
        let f = fixture();
        let mut harness = harness(f.state);
        harness.get_by_label("orders").click_modifiers(modifiers);
        harness.run();
        let (session, action) = selection_action(harness.state_mut().1.take());
        assert_eq!(session, f.active);
        match action {
            SidebarAction::Select { node, mode } => {
                assert_eq!(mode, expected);
                assert!(
                    matches!(node, NodeRef::Collection { ns, .. } if ns.collection == "orders")
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn a_right_click_selects_an_unselected_row_alone() {
    let f = fixture();
    let mut harness = harness(f.state);
    harness.get_by_label("orders").click_secondary();
    harness.run();
    let (_, action) = selection_action(harness.state_mut().1.take());
    assert!(matches!(
        action,
        SidebarAction::Select {
            mode: SelectMode::Single,
            ..
        }
    ));
}

#[test]
fn a_row_menu_lists_copy_and_paste_disabled_when_there_is_nothing_to_do() {
    let i18n = I18n::english();
    let mut harness = harness(fixture().state);
    harness
        .get_by_label(&db_row_label("shop", true))
        .click_secondary();
    harness.run();
    harness.state_mut().1.take();
    // Nothing is selected and nothing was copied: both entries are listed, and clicking does
    // nothing.
    harness.get_by_label(&i18n.t("copy.menu.copy")).click();
    harness.get_by_label(&i18n.t("copy.paste")).click();
    harness.run();
    assert!(harness.state_mut().1.take().is_none());
}

#[test]
fn a_row_menu_names_the_count_and_copies_the_selection() {
    let i18n = I18n::english();
    let mut f = fixture();
    let a = f.active;
    let node = NodeRef::Collection {
        session: a,
        ns: Namespace::new("shop", "orders"),
    };
    f.state.selection = SidebarSelection::default().single(node);
    let mut harness = harness(f.state);
    harness
        .get_by_label(&db_row_label("shop", true))
        .click_secondary();
    harness.run();
    harness.state_mut().1.take();
    let label = i18n.t_with("copy.menu.copy_collection", &[("count", "1")]);
    harness.get_by_label(&label).click();
    harness.run();
    let (session, action) = selection_action(harness.state_mut().1.take());
    assert_eq!((session, action), (a, SidebarAction::Copy));
}

#[test]
fn a_row_menu_pastes_onto_the_row_when_the_rules_allow_it() {
    let i18n = I18n::english();
    let mut f = fixture();
    let a = f.active;
    f.state.clipboard = CopyClipboard::collections(a, "shop", vec!["orders".into()]).ok();
    let mut harness = harness(f.state);
    harness
        .get_by_label(&db_row_label("shop", true))
        .click_secondary();
    harness.run();
    harness.state_mut().1.take();
    harness.get_by_label(&i18n.t("copy.paste")).click();
    harness.run();
    let (_, action) = selection_action(harness.state_mut().1.take());
    assert!(matches!(
        action,
        SidebarAction::Paste(PasteTarget::Database { db, .. }) if db == "shop"
    ));
}

#[test]
fn copy_and_paste_keys_act_on_the_focused_row_only() {
    let mut f = fixture();
    let a = f.active;
    f.state.clipboard = CopyClipboard::collections(a, "shop", vec!["orders".into()]).ok();
    let mut harness = harness(f.state);
    harness.get_by_label("orders").focus();
    harness.run();
    harness.state_mut().1.take();
    harness.input_mut().events.push(egui::Event::Copy);
    harness.run();
    let (_, action) = selection_action(harness.state_mut().1.take());
    assert_eq!(action, SidebarAction::Copy);
    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(String::new()));
    harness.run();
    let (_, action) = selection_action(harness.state_mut().1.take());
    assert!(matches!(
        action,
        SidebarAction::Paste(PasteTarget::Database { db, .. }) if db == "shop"
    ));
}

#[test]
fn the_keys_do_nothing_when_no_row_has_focus() {
    let mut harness = harness(fixture().state);
    harness.input_mut().events.push(egui::Event::Copy);
    harness.run();
    assert!(harness.state_mut().1.take().is_none());
}

#[test]
fn a_roots_menu_and_key_paste_onto_the_connection() {
    let mut f = fixture();
    let a = f.active;
    f.state.clipboard = CopyClipboard::database(a, "shop").ok();
    let beta = f.parked;
    let mut harness = harness(f.state);
    harness.get_by_label("Beta").focus();
    harness.run();
    harness.state_mut().1.take();
    harness
        .input_mut()
        .events
        .push(egui::Event::Paste(String::new()));
    harness.run();
    match harness.state_mut().1.take() {
        Some(Input::Sidebar(SidebarAction::Paste(PasteTarget::Connection { session }))) => {
            assert_eq!(session, beta);
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// Focus on a row, then a key press with `modifiers` that moves the focus; returns what the
/// sidebar emitted for the move.
fn focus_move(key: egui::Key, modifiers: egui::Modifiers) -> SidebarAction {
    let mut harness = harness(fixture().state);
    harness.get_by_label("orders").focus();
    harness.run();
    harness.state_mut().1.take();
    harness.input_mut().modifiers = modifiers;
    harness.input_mut().events.push(egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    });
    harness.run();
    selection_action(harness.state_mut().1.take()).1
}

#[test]
fn shift_tab_onto_a_row_selects_it_alone() {
    match focus_move(egui::Key::Tab, egui::Modifiers::SHIFT) {
        SidebarAction::Select { node, mode } => {
            assert_eq!(mode, SelectMode::Single);
            assert!(matches!(node, NodeRef::Database { db, .. } if db == "shop"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn shift_arrow_still_extends_the_selection() {
    // The focus moves by an arrow key with Shift held (no Tab press): the row is added as a
    // range. (egui lands an arrow's focus move a frame late; moving the focus directly keeps
    // the test about the sidebar's reading of the modifiers.)
    let mut harness = harness(fixture().state);
    harness.get_by_label("orders").focus();
    harness.run();
    harness.state_mut().1.take();
    harness.input_mut().modifiers = egui::Modifiers::SHIFT;
    harness.get_by_label(&db_row_label("shop", true)).focus();
    harness.run();
    match selection_action(harness.state_mut().1.take()).1 {
        SidebarAction::Select { mode, .. } => assert_eq!(mode, SelectMode::Extend),
        other => panic!("unexpected {other:?}"),
    }
}
