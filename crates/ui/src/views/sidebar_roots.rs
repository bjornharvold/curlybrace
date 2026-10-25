//! The connection tree: one collapsible root per open session. A connected root holds that
//! session's database tree; a failed root holds its error; a connecting root only its status.
use crate::I18n;
use crate::app::reducer::Input;
use crate::app::session::Root;
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::SidebarAction;
use crate::views::connection_badge::show_badge;
use crate::views::sidebar::{preferred_body_height, show_sidebar_body};
use crate::views::sidebar_copy::{TreeCtx, root_action};
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
    let header = ui.vertical(|ui| show_header(ui, &mut collapse, state, root, i18n));
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
    state: &AppState,
    root: &Root,
    i18n: &I18n,
) -> Option<Input> {
    let title = ui
        .horizontal(|ui| show_title_line(ui, collapse, state, root, i18n))
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
    state: &AppState,
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
            let name = ui.selectable_label(root.active, &root.name);
            let paste = root_action(&name, ui, state, root.id, i18n);
            if let Some(action) = paste {
                input = Some(Input::Sidebar(action));
            } else if name.clicked() {
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
    let ctx = TreeCtx {
        state,
        session: root.id,
    };
    let action = ui
        .allocate_ui(size, |ui| show_sidebar_body(ui, sidebar, Some(&ctx), i18n))
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
#[path = "sidebar_roots_tests.rs"]
mod tests;
