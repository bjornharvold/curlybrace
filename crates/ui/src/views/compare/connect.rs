//! Which saved connections the two sides read from, and opening/closing the session.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, SessionPhase};
use crate::views::connections::ConnectionsState;
use eframe::egui;
use uuid::Uuid;

/// The picks being made before they are opened; a widget buffer, kept in egui memory.
type Draft = (Option<Uuid>, Option<Uuid>);

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
    let mut draft = ui
        .data(|d| d.get_temp::<Draft>(id))
        .unwrap_or_else(|| initial_draft(state, connections));
    ui.horizontal_wrapped(|ui| {
        ui.push_id("source", |ui| {
            let key = "compare.session.source_connection";
            connection_combo(ui, key, &mut draft.0, connections, i18n);
        });
        ui.push_id("target", |ui| {
            let key = "compare.session.target_connection";
            connection_combo(ui, key, &mut draft.1, connections, i18n);
        });
    });
    ui.data_mut(|d| d.insert_temp(id, draft));
    session_buttons(ui, state, draft, i18n)
}

/// What the pickers show before the user touched them: the open session's connections, else the
/// one selected in the connection manager, else the first saved one.
fn initial_draft(state: &CompareState, connections: &ConnectionsState) -> Draft {
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
    draft: Draft,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    let opening = matches!(state.session, Some(SessionPhase::Opening(_)));
    let changed = state.connections != draft.0.zip(draft.1);
    ui.horizontal(|ui| {
        let open = egui::Button::new(i18n.t("compare.session.open"));
        let can_open = draft.0.is_some() && draft.1.is_some() && changed && !opening;
        if ui.add_enabled(can_open, open).clicked() {
            action = draft
                .0
                .zip(draft.1)
                .map(|(source, target)| CompareAction::Open { source, target });
        }
        let close = egui::Button::new(i18n.t("compare.session.close"));
        if ui.add_enabled(state.session.is_some(), close).clicked() {
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
