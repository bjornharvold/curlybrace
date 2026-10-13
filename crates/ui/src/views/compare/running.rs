//! Start / Cancel, the progress counters of a run and its outcome.
use super::duplicates::show_duplicates;
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, RunView};
use curlybrace_core::error::CoreError;
use eframe::egui;

/// `setup_ready`: the setup has no issues (computed once per frame by the caller).
pub fn show_run(
    ui: &mut egui::Ui,
    state: &CompareState,
    setup_ready: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let start = egui::Button::new(i18n.t("compare.run.start"));
        if ui
            .add_enabled(state.can_start_given(setup_ready), start)
            .clicked()
        {
            action = Some(CompareAction::Start);
        }
        let cancel = egui::Button::new(i18n.t("compare.run.cancel"));
        if ui.add_enabled(state.can_cancel(), cancel).clicked() {
            action = Some(CompareAction::Cancel);
        }
        let discard = egui::Button::new(i18n.t("compare.run.discard"));
        if ui.add_enabled(state.can_discard(), discard).clicked() {
            action = Some(CompareAction::Discard);
        }
    });
    let duplicates = show_progress(ui, state, i18n);
    action.or(duplicates)
}

fn scanned_text(source: u64, target: u64, i18n: &I18n) -> String {
    i18n.t_with(
        "compare.run.scanned",
        &[
            ("source", &source.to_string()),
            ("target", &target.to_string()),
        ],
    )
}

fn show_progress(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let cancel_requested = state.cancel_requested;
    match &state.run {
        RunView::Idle => {}
        RunView::Running {
            scanned_source,
            scanned_target,
            differences,
            cancelled,
            failure,
            ..
        } => {
            let scanned = scanned_text(*scanned_source, *scanned_target, i18n);
            // The counters sit under the bar: the animated knob would overlap text drawn on it.
            ui.add(egui::ProgressBar::new(0.0).animate(true));
            ui.label(scanned);
            let found = differences.to_string();
            ui.label(i18n.t_with("compare.run.differences", &[("count", &found)]));
            if *cancelled || cancel_requested {
                ui.label(i18n.t("compare.run.cancelling"));
            }
            show_failure(ui, failure.as_ref(), i18n);
        }
        RunView::Finished {
            cancelled,
            failure,
            scanned_source,
            scanned_target,
            ..
        } => {
            // A failed run did not finish: only the failure line speaks for it.
            if *cancelled {
                ui.label(i18n.t("compare.run.cancelled_no_results"));
            } else if failure.is_none() {
                ui.label(i18n.t("compare.run.finished"));
            }
            ui.label(scanned_text(*scanned_source, *scanned_target, i18n));
            show_failure(ui, failure.as_ref(), i18n);
        }
    }
    show_duplicates(ui, state, i18n)
}

fn show_failure(ui: &mut egui::Ui, failure: Option<&CoreError>, i18n: &I18n) {
    let Some(error) = failure else {
        return;
    };
    let color = ui.visuals().error_fg_color;
    ui.colored_label(color, i18n.t("compare.run.failed"));
    ui.colored_label(color, i18n.error(error));
}
