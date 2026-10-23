//! The connection tree: one collapsible root per open session. A connected root holds that
//! session's database tree; a failed root holds its error; a connecting root only its status.
use crate::I18n;
use crate::app::reducer::Input;
use crate::app::session::Root;
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::SidebarAction;
use crate::views::connection_badge::show_badge;
use crate::views::sidebar::{preferred_body_height, show_sidebar_body};
use curlybrace_core::store::Topology;
use eframe::egui;
use eframe::egui::collapsing_header::CollapsingState;
use eframe::egui::{WidgetInfo, WidgetType};

/// Rows of a root's database tree shown before it scrolls.
const MAX_TREE_ROWS: usize = 12;
/// Narrowest the root name's area gets, however wide the Disconnect button is.
const MIN_NAME_WIDTH: f32 = 76.0;

/// The Connections button followed by one root per open session. Returns what the user did.
pub fn show_roots(ui: &mut egui::Ui, state: &AppState, i18n: &I18n) -> Option<Input> {
    let mut input = ui
        .button(i18n.t("sidebar.connections"))
        .clicked()
        .then_some(Input::ShowConnections);
    for root in state.roots(i18n) {
        input = ui
            .push_id(("sidebar_root", root.id), |ui| {
                show_root(ui, state, &root, i18n)
            })
            .inner
            .or(input);
    }
    input
}

fn show_root(ui: &mut egui::Ui, state: &AppState, root: &Root, i18n: &I18n) -> Option<Input> {
    let id = ui.make_persistent_id("collapse");
    let mut collapse = CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let header = ui.vertical(|ui| show_header(ui, &mut collapse, root, i18n));
    let body = collapse
        .show_body_indented(&header.response, ui, |ui| show_body(ui, state, root, i18n))
        .and_then(|shown| shown.inner);
    header.inner.or(body)
}

/// Two lines, and a third while the root has a colour tag or the production flag. Line 1: toggle, the name (wraps inside its own area, never truncated) and the
/// Disconnect right-aligned.
/// Line 2 (weak): status and, when known, topology, and Reconnect right-aligned on a failed root.
/// Line 3 (tagged roots only): the colour glyph and the PRODUCTION badge, on a line of their own
/// so they never widen the sidebar past what the title and status lines need.
fn show_header(
    ui: &mut egui::Ui,
    collapse: &mut CollapsingState,
    root: &Root,
    i18n: &I18n,
) -> Option<Input> {
    let title = ui
        .horizontal(|ui| show_title_line(ui, collapse, root, i18n))
        .inner;
    let status = ui.horizontal(|ui| show_status_line(ui, root, i18n)).inner;
    show_tag_line(ui, root, i18n);
    status.or(title)
}

/// The status (and topology) on the left; on a failed root Reconnect on the right, so the
/// first line never needs room for more than the toggle, the name and Disconnect.
fn show_status_line(ui: &mut egui::Ui, root: &Root, i18n: &I18n) -> Option<Input> {
    ui.label(egui::RichText::new(status_line(root, i18n)).weak());
    if root.status != ConnStatus::Failed {
        return None;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let name = [("name", root.name.as_str())];
        let reconnect = ui.small_button(i18n.t("sidebar.reconnect"));
        named(&reconnect, i18n.t_with("sidebar.reconnect_named", &name));
        reconnect.clicked().then_some(Input::Connect(root.id))
    })
    .inner
}

fn show_tag_line(ui: &mut egui::Ui, root: &Root, i18n: &I18n) {
    let badge = &root.badge;
    if badge.color.is_none() && !badge.production && !badge.read_only {
        return;
    }
    ui.horizontal_wrapped(|ui| show_badge(ui, badge, i18n));
}

fn show_title_line(
    ui: &mut egui::Ui,
    collapse: &mut CollapsingState,
    root: &Root,
    i18n: &I18n,
) -> Option<Input> {
    let name = [("name", root.name.as_str())];
    let arrow = if collapse.is_open() { "▾" } else { "▸" };
    let toggle = ui.small_button(arrow);
    named(&toggle, i18n.t_with("sidebar.toggle_named", &name));
    if toggle.clicked() {
        collapse.toggle(ui);
    }
    let disconnect_text = i18n.t("connections.disconnect");
    // The name takes what Disconnect leaves, so it stays on this line and the name wraps.
    let buttons = small_button_width(ui, &disconnect_text);
    let name_width =
        (ui.available_width() - buttons - ui.spacing().item_spacing.x).max(MIN_NAME_WIDTH);
    let mut input = None;
    // One line tall (centred on the line like the buttons); a wrapping name grows downwards.
    ui.allocate_ui_with_layout(
        egui::vec2(name_width, ui.spacing().interact_size.y),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(name_width);
            if ui.selectable_label(root.active, &root.name).clicked() {
                input = Some(Input::Activate(root.id));
            }
        },
    );
    let disconnect = ui.small_button(disconnect_text);
    named(&disconnect, i18n.t_with("sidebar.disconnect_named", &name));
    if disconnect.clicked() {
        input = Some(Input::Disconnect(root.id));
    }
    input
}

/// Width `ui.small_button(text)` will take.
fn small_button_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().text_color();
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    galley.size().x + 2.0 * ui.spacing().button_padding.x
}

/// "Connected · Replica set", or just the status while the topology is unknown.
fn status_line(root: &Root, i18n: &I18n) -> String {
    let status = i18n.t(status_key(root.status));
    match root.topology {
        Some(topology) => i18n.t_with(
            "sidebar.status_topology",
            &[
                ("status", status.as_str()),
                ("topology", i18n.t(topology_key(topology)).as_str()),
            ],
        ),
        None => status,
    }
}

/// Gives a button an accessible name that says which connection it acts on.
fn named(response: &egui::Response, label: String) {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
}

fn show_body(ui: &mut egui::Ui, state: &AppState, root: &Root, i18n: &I18n) -> Option<Input> {
    match root.status {
        ConnStatus::Connected => show_tree(ui, state, root, i18n),
        ConnStatus::Failed => {
            let color = ui.visuals().error_fg_color;
            ui.colored_label(color, i18n.t("sidebar.root_failed"));
            if let Some(error) = &root.failure {
                ui.label(i18n.error(error));
            }
            None
        }
        ConnStatus::Connecting | ConnStatus::Disconnected => None,
    }
}

/// The root's database tree in a bounded area; its actions are tagged with the root's session
/// so the reducer routes them whether the root is active or parked.
fn show_tree(ui: &mut egui::Ui, state: &AppState, root: &Root, i18n: &I18n) -> Option<Input> {
    let sidebar = if root.active {
        &state.sidebar
    } else {
        &state.parked.get(&root.id)?.sidebar
    };
    let size = egui::vec2(
        ui.available_width(),
        preferred_body_height(ui, sidebar, MAX_TREE_ROWS),
    );
    let action = ui
        .allocate_ui(size, |ui| show_sidebar_body(ui, sidebar, i18n))
        .inner?;
    Some(Input::Sidebar(SidebarAction::InSession {
        session: root.id,
        action: Box::new(action),
    }))
}

fn status_key(status: ConnStatus) -> &'static str {
    match status {
        ConnStatus::Connected => "sidebar.status_connected",
        ConnStatus::Connecting => "sidebar.status_connecting",
        ConnStatus::Failed => "sidebar.status_failed",
        // An open root is never idle; keep the label honest if it ever is.
        ConnStatus::Disconnected => "sidebar.status_failed",
    }
}

fn topology_key(topology: Topology) -> &'static str {
    match topology {
        Topology::Direct => "sidebar.topology_direct",
        Topology::ReplicaSet => "sidebar.topology_replica_set",
        Topology::Sharded => "sidebar.topology_sharded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::SessionView;
    use crate::viewmodel::SidebarState;
    use crate::viewmodel::connection_tag::PRODUCTION_BADGE_KEY;
    use curlybrace_core::connection::ConnectionConfig;
    use curlybrace_core::error::CoreError;
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
}
