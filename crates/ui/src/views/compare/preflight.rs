//! The preflight strip: the Check button, what the last check found, and what blocks a run.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, PreflightView};
use eframe::egui;

pub fn show_preflight(
    ui: &mut egui::Ui,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        // While a check is in flight another cannot start, and an edit meanwhile leaves the
        // answer stale: the button says so instead of silently doing nothing.
        let label = if state.check_in_flight {
            "compare.preflight.checking"
        } else {
            "compare.preflight.check"
        };
        let button = egui::Button::new(i18n.t(label));
        if ui.add_enabled(state.can_check(), button).clicked() {
            action = Some(CompareAction::RunCheck);
        }
        if state.check_in_flight {
            ui.spinner();
        }
        ui.label(i18n.t(status_key(&state.preflight)));
    });
    show_message(ui, &state.preflight, i18n);
    action
}

fn status_key(preflight: &PreflightView) -> &'static str {
    match preflight {
        PreflightView::Unchecked => "compare.preflight.unchecked",
        PreflightView::Checking => "compare.preflight.checking",
        PreflightView::Ok => "compare.preflight.ok",
        PreflightView::NeedsIndex { .. } => "compare.preflight.blocked",
        PreflightView::EmptyBoth => "compare.preflight.empty",
    }
}

/// The explanation with its next step, in the error colour when it blocks the run.
fn show_message(ui: &mut egui::Ui, preflight: &PreflightView, i18n: &I18n) {
    let Some(message) = preflight.message() else {
        return;
    };
    let text = match message.side_key {
        Some(side) => i18n.t_with(message.key, &[("side", &i18n.t(side))]),
        None => i18n.t(message.key),
    };
    if preflight.allows_start() {
        ui.label(text);
    } else {
        let color = ui.visuals().error_fg_color;
        ui.colored_label(color, text);
    }
}
