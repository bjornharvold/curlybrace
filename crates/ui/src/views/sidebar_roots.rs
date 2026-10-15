//! The connection tree: one collapsible root per open session. A connected root holds that
//! session's database tree; a failed root holds its error; a connecting root only its status.
use crate::I18n;
use crate::app::reducer::Input;
use crate::app::session::Root;
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::SidebarAction;
use crate::views::sidebar::{preferred_height, show_sidebar};
use curlybrace_core::store::Topology;
use eframe::egui;
use eframe::egui::collapsing_header::CollapsingState;
use eframe::egui::{WidgetInfo, WidgetType};

/// Rows of a root's database tree shown before it scrolls.
const MAX_TREE_ROWS: usize = 12;

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
    let header = ui.horizontal_wrapped(|ui| show_header(ui, &mut collapse, root, i18n));
    let body = collapse
        .show_body_indented(&header.response, ui, |ui| show_body(ui, state, root, i18n))
        .and_then(|shown| shown.inner);
    header.inner.or(body)
}

/// Toggle, name (selects the root), status, topology, Disconnect and, when failed, Reconnect.
fn show_header(
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
    let mut input = None;
    if ui.selectable_label(root.active, &root.name).clicked() {
        input = Some(Input::Activate(root.id));
    }
    ui.weak(i18n.t(status_key(root.status)));
    if let Some(topology) = root.topology {
        ui.weak(i18n.t(topology_key(topology)));
    }
    let disconnect = ui.small_button(i18n.t("connections.disconnect"));
    named(&disconnect, i18n.t_with("sidebar.disconnect_named", &name));
    if disconnect.clicked() {
        input = Some(Input::Disconnect(root.id));
    }
    if root.status == ConnStatus::Failed {
        let reconnect = ui.small_button(i18n.t("sidebar.reconnect"));
        named(&reconnect, i18n.t_with("sidebar.reconnect_named", &name));
        if reconnect.clicked() {
            input = Some(Input::Connect(root.id));
        }
    }
    input
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
        preferred_height(ui, sidebar, MAX_TREE_ROWS),
    );
    let action = ui
        .allocate_ui(size, |ui| show_sidebar(ui, sidebar, i18n))
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
    use curlybrace_core::connection::ConnectionConfig;
    use curlybrace_core::error::CoreError;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use uuid::Uuid;

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
}
