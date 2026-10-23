//! The Compare workspace: the pair list set up, checked, run and browsed on one page. Every
//! decision (what is enabled, what a click means) lives in `viewmodel::compare` and the
//! reducer; these views render the state and report the user's action.
mod apply;
mod apply_result;
mod connect;
mod detail;
mod duplicates;
mod editor;
mod export;
mod export_result;
mod fields;
mod memo;
mod overview;
mod pairs;
mod pending;
mod preflight;
mod results;
mod rules;
mod select;
mod setup;
mod switcher;
mod table;

use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, ListInputs, ListView, SessionPhase};
use crate::views::connections::ConnectionsState;
use eframe::egui;

pub use connect::show_connect;
pub use detail::show_detail;
pub use duplicates::show_duplicates;
pub use editor::show_editor;
pub use overview::show_overview;
pub use pairs::show_pairs;
pub use preflight::show_preflight;
pub use results::{show_pair_results, show_results};
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
        Some(SessionPhase::Ready(_)) => show_ready(ui, state, connections, i18n),
    };
    let production = |id| {
        connections
            .connections
            .iter()
            .any(|c| c.id == id && c.production)
    };
    let review = apply::show_review(ui.ctx(), state, &production, i18n);
    let export = export::show_export(ui.ctx(), state, i18n);
    header.or(connect).or(body).or(review).or(export)
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

/// Everything below the connection pickers, once the session is usable: the results of the
/// open pair, or else setup, pairs, preflight and the run overview.
fn show_ready(
    ui: &mut egui::Ui,
    state: &CompareState,
    connections: &ConnectionsState,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.separator();
    let progress = apply::show_progress(ui, state, i18n);
    let rest = if state.open_pair.is_some() {
        let results = show_pair_results(ui, state, i18n);
        let pending = pending::show_pending(ui, state, connections, i18n);
        results.or(pending)
    } else {
        show_setup_and_runs(ui, state, connections, i18n)
    };
    progress.or(rest)
}

fn show_setup_and_runs(
    ui: &mut egui::Ui,
    state: &CompareState,
    connections: &ConnectionsState,
    i18n: &I18n,
) -> Option<CompareAction> {
    detail::forget_cache(ui);
    // The per-entry work (parsing keys and filters, the pairing helper) happens once per
    // change of the list or the defaults, not once per frame.
    let list = list_view(ui, state);
    let gates = state.gates(&list.digest);
    let setup = show_setup(ui, state, i18n);
    ui.separator();
    let pairs = show_pairs(ui, state, &list, i18n);
    let editor = show_editor(ui, state, i18n);
    ui.separator();
    let preflight = show_preflight(ui, state, &gates, &list, i18n);
    let overview = show_overview(ui, state, i18n);
    let pending = pending::show_pending(ui, state, connections, i18n);
    setup
        .or(pairs)
        .or(editor)
        .or(preflight)
        .or(overview)
        .or(pending)
}

fn list_view(ui: &egui::Ui, state: &CompareState) -> std::sync::Arc<ListView> {
    memo::memo(
        ui,
        "list_view",
        |inputs: &ListInputs| inputs.is_for(state),
        || (ListInputs::of(state), ListView::of(state)),
    )
}
