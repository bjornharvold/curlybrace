//! Which saved connections the two sides read from, and opening/closing the session.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::theme::{filled_button, gate, tonal_button};
use crate::viewmodel::compare::CompareState;
use crate::viewmodel::connection_tag::READ_ONLY_BADGE_KEY;
use crate::views::connections::ConnectionsState;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::connection::ConnectionConfig;
use eframe::egui;
use uuid::Uuid;

/// The picks being made before they are opened; a widget buffer, kept in egui memory.
type Picks = (Option<Uuid>, Option<Uuid>);
/// The picks tagged with the open pair they were derived from: when the session's pair changes
/// behind the view's back (the sidebar's "Compare to...") the old picks are dropped.
type Draft = (Option<(Uuid, Uuid)>, Picks);

/// One wrapping row: what `leading` draws (the page's heading and Back), the two connection
/// pickers and Open / Close; the read-only note under it. Returns the first action of the
/// leading part, the pickers' buttons otherwise.
pub fn show_connect(
    ui: &mut egui::Ui,
    state: &CompareState,
    connections: &ConnectionsState,
    i18n: &I18n,
    leading: impl FnOnce(&mut egui::Ui) -> Option<CompareAction>,
) -> Option<CompareAction> {
    if connections.connections.is_empty() {
        let led = ui.horizontal_wrapped(leading).inner;
        ui.label(i18n.t("compare.session.no_connections"));
        return led;
    }
    let id = ui.make_persistent_id("compare_connection_draft");
    let (_, mut picks) = ui
        .data(|d| d.get_temp::<Draft>(id))
        .filter(|(origin, _)| *origin == state.connections)
        .unwrap_or_else(|| (state.connections, initial_picks(state, connections)));
    // A compare can write to its target, so a read-only connection is never a target pick (a
    // stale or default pick of one is cleared); it stays listed, disabled, with the reason.
    picks.1 = picks.1.filter(|id| !is_read_only(connections, *id));
    let action = ui
        .horizontal_wrapped(|ui| {
            let led = leading(ui);
            wrapped_group(ui, "source", |ui| {
                let key = "compare.session.source_connection";
                connection_combo(ui, key, Side::Source, &mut picks.0, connections, i18n);
            });
            wrapped_group(ui, "target", |ui| {
                let key = "compare.session.target_connection";
                connection_combo(ui, key, Side::Target, &mut picks.1, connections, i18n);
            });
            led.or(session_buttons(ui, state, picks, i18n))
        })
        .inner;
    if connections.connections.iter().any(|c| c.read_only) {
        ui.label(i18n.t(TARGET_READ_ONLY_KEY));
    }
    ui.data_mut(|d| d.insert_temp::<Draft>(id, (state.connections, picks)));
    action
}

/// Draws `add` (a picker and its caption, which egui lays out as one group it cannot wrap) under
/// the id `salt`, starting a new line of the wrapping row first when the group's width from the
/// last frame does not fit in the rest of this one. A changed width redoes the frame, so a group
/// is never shown past the edge.
fn wrapped_group(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    let key = ui.id().with((salt, "group_width"));
    let last = ui.data(|d| d.get_temp::<f32>(key));
    let line_started = ui.cursor().min.x > ui.max_rect().min.x + 0.5;
    if line_started && last.is_some_and(|width| width > ui.available_size_before_wrap().x) {
        ui.end_row();
    }
    let width = ui.push_id(salt, add).response.rect.width();
    if last.is_none_or(|last| (last - width).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(key, width));
        ui.ctx().request_discard("compare picker measured");
    }
}

const TARGET_READ_ONLY_KEY: &str = "compare.session.target_read_only";

fn is_read_only(connections: &ConnectionsState, id: Uuid) -> bool {
    connections
        .connections
        .iter()
        .any(|c| c.id == id && c.read_only)
}

/// The name as the pickers list it: read-only connections carry the badge text.
fn option_label(cfg: &ConnectionConfig, i18n: &I18n) -> String {
    match cfg.read_only {
        true => i18n.t_with(
            "connection.name_with_read_only",
            &[("name", &cfg.name), ("badge", &i18n.t(READ_ONLY_BADGE_KEY))],
        ),
        false => cfg.name.clone(),
    }
}

/// What the pickers show before the user touched them: the open session's connections, else the
/// one selected in the connection manager, else the first saved one.
fn initial_picks(state: &CompareState, connections: &ConnectionsState) -> Picks {
    if let Some((source, target)) = state.connections {
        return (Some(source), Some(target));
    }
    let fallback = connections
        .selected
        .or_else(|| connections.connections.first().map(|c| c.id));
    (fallback, fallback)
}

/// Open and Close, in the caller's row.
fn session_buttons(
    ui: &mut egui::Ui,
    state: &CompareState,
    picks: Picks,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    let open = gate(ui, state.can_open(picks), |ui| {
        filled_button(ui, i18n.t("compare.session.open"))
    });
    if open.clicked() {
        action = picks
            .0
            .zip(picks.1)
            .map(|(source, target)| CompareAction::Open { source, target });
    }
    let close = gate(ui, state.can_close(), |ui| {
        tonal_button(ui, i18n.t("compare.session.close"))
    });
    if close.clicked() {
        action = Some(CompareAction::CloseSession);
    }
    action
}

fn connection_combo(
    ui: &mut egui::Ui,
    label_key: &str,
    side: Side,
    value: &mut Option<Uuid>,
    connections: &ConnectionsState,
    i18n: &I18n,
) {
    let current = connections
        .connections
        .iter()
        .find(|c| Some(c.id) == *value)
        .map(|c| option_label(c, i18n))
        .unwrap_or_else(|| i18n.t("compare.session.pick_connection"));
    egui::ComboBox::new("connection", i18n.t(label_key))
        .selected_text(current)
        .show_ui(ui, |ui| {
            for cfg in &connections.connections {
                let selected = *value == Some(cfg.id);
                let locked = side == Side::Target && cfg.read_only;
                let label = option_label(cfg, i18n);
                let row = ui.push_id(cfg.id, |ui| {
                    ui.add_enabled(!locked, egui::Button::selectable(selected, label))
                });
                if locked {
                    row.inner
                        .on_disabled_hover_text(i18n.t(TARGET_READ_ONLY_KEY));
                } else if row.inner.clicked() {
                    *value = Some(cfg.id);
                }
            }
        });
}
