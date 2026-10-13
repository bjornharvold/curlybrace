//! The Compare workspace: one pair, set up, checked, run and browsed on a single page. Every
//! decision (what is enabled, what a click means) lives in `viewmodel::compare` and the
//! reducer; these views render the state and report the user's action.
mod connect;
mod detail;
mod duplicates;
mod preflight;
mod results;
mod rules;
mod running;
mod setup;
mod switcher;

use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, SessionPhase};
use crate::views::connections::ConnectionsState;
use eframe::egui;

pub use connect::show_connect;
pub use detail::show_detail;
pub use duplicates::show_duplicates;
pub use preflight::show_preflight;
pub use results::show_results;
pub use running::show_run;
pub use setup::show_setup;
pub use switcher::show_workspace_switcher;

/// The whole workspace. Returns the action the user took this frame, if any.
pub fn show_compare(
    ui: &mut egui::Ui,
    state: &CompareState,
    connections: &ConnectionsState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("compare_workspace")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            action = layout(ui, state, connections, i18n);
        });
    action
}

fn layout(
    ui: &mut egui::Ui,
    state: &CompareState,
    connections: &ConnectionsState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let header = show_header(ui, i18n);
    let connect = show_connect(ui, state, connections, i18n);
    let body = match state.session {
        None => None,
        Some(SessionPhase::Opening(_)) => {
            ui.label(i18n.t("compare.session.opening"));
            None
        }
        Some(SessionPhase::Ready(_)) => show_ready(ui, state, i18n),
    };
    header.or(connect).or(body)
}

fn show_header(ui: &mut egui::Ui, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.heading(i18n.t("compare.title"));
        if ui.button(i18n.t("compare.back")).clicked() {
            action = Some(CompareAction::Hide);
        }
    });
    action
}

/// Everything below the connection pickers, once the session is usable.
fn show_ready(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    ui.separator();
    // Parsing the form is not free: its issues are computed once per frame.
    let issues = state.issues();
    let setup_ready = issues.is_empty();
    let setup = show_setup(ui, &state.setup, &issues, i18n);
    ui.separator();
    let preflight = show_preflight(ui, state, setup_ready, i18n);
    let run = show_run(ui, state, i18n);
    let lower = show_lower(ui, state, i18n);
    setup.or(preflight).or(run).or(lower)
}

/// Results and the open detail; the detail's cached JSON is dropped once it is gone.
fn show_lower(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let Some(results) = state.open_results() else {
        detail::forget_cache(ui);
        return None;
    };
    ui.separator();
    let selected = state.detail.as_ref().map(|d| &d.key);
    let table = show_results(ui, results, selected, i18n);
    let detail = match &state.detail {
        Some(d) => {
            ui.separator();
            show_detail(ui, d, i18n)
        }
        None => {
            detail::forget_cache(ui);
            None
        }
    };
    table.or(detail)
}
