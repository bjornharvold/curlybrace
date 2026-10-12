//! Which saved connections the two sides read from, and opening/closing the session.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::CompareState;
use crate::views::connections::ConnectionsState;
use eframe::egui;
use uuid::Uuid;

/// The picks being made before they are opened; a widget buffer, kept in egui memory.
type Picks = (Option<Uuid>, Option<Uuid>);
/// The picks tagged with the open pair they were derived from: when the session's pair changes
/// behind the view's back (the sidebar's "Compare to...") the old picks are dropped.
type Draft = (Option<(Uuid, Uuid)>, Picks);

pub fn show_connect(
    ui: &mut egui::Ui,
    state: &CompareState,
    connections: &ConnectionsState,
    i18n: &I18n,
) -> Option<CompareAction> {
    if connections.connections.is_empty() {
        ui.label(i18n.t("compare.session.no_connections"));
        return None;
    }
    let id = ui.make_persistent_id("compare_connection_draft");
    let (_, mut picks) = ui
        .data(|d| d.get_temp::<Draft>(id))
        .filter(|(origin, _)| *origin == state.connections)
        .unwrap_or_else(|| (state.connections, initial_picks(state, connections)));
    ui.horizontal_wrapped(|ui| {
        ui.push_id("source", |ui| {
            let key = "compare.session.source_connection";
            connection_combo(ui, key, &mut picks.0, connections, i18n);
        });
        ui.push_id("target", |ui| {
            let key = "compare.session.target_connection";
            connection_combo(ui, key, &mut picks.1, connections, i18n);
        });
    });
    ui.data_mut(|d| d.insert_temp::<Draft>(id, (state.connections, picks)));
    session_buttons(ui, state, picks, i18n)
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

fn session_buttons(
    ui: &mut egui::Ui,
    state: &CompareState,
    picks: Picks,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let open = egui::Button::new(i18n.t("compare.session.open"));
        if ui.add_enabled(state.can_open(picks), open).clicked() {
            action = picks
                .0
                .zip(picks.1)
                .map(|(source, target)| CompareAction::Open { source, target });
        }
        let close = egui::Button::new(i18n.t("compare.session.close"));
        if ui.add_enabled(state.can_close(), close).clicked() {
            action = Some(CompareAction::CloseSession);
        }
    });
    action
}

fn connection_combo(
    ui: &mut egui::Ui,
    label_key: &str,
    value: &mut Option<Uuid>,
    connections: &ConnectionsState,
    i18n: &I18n,
) {
    let current = connections
        .connections
        .iter()
        .find(|c| Some(c.id) == *value)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| i18n.t("compare.session.pick_connection"));
    egui::ComboBox::new("connection", i18n.t(label_key))
        .selected_text(current)
        .show_ui(ui, |ui| {
            for cfg in &connections.connections {
                let selected = *value == Some(cfg.id);
                let row = ui.push_id(cfg.id, |ui| ui.selectable_label(selected, &cfg.name));
                if row.inner.clicked() {
                    *value = Some(cfg.id);
                }
            }
        });
}
