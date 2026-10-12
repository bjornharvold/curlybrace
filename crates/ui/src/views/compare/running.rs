//! Start / Cancel, the progress counters of a run and its outcome.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, RunView};
use curlybrace_core::error::CoreError;
use eframe::egui;

pub fn show_run(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let start = egui::Button::new(i18n.t("compare.run.start"));
        if ui.add_enabled(state.can_start(), start).clicked() {
            action = Some(CompareAction::Start);
        }
        let cancel = egui::Button::new(i18n.t("compare.run.cancel"));
        if ui.add_enabled(state.is_running(), cancel).clicked() {
            action = Some(CompareAction::Cancel);
        }
        let finished = matches!(state.run, RunView::Finished { .. });
        let discard = egui::Button::new(i18n.t("compare.run.discard"));
        if ui.add_enabled(finished, discard).clicked() {
            action = Some(CompareAction::Discard);
        }
    });
    show_progress(ui, &state.run, i18n);
    action
}

fn show_progress(ui: &mut egui::Ui, run: &RunView, i18n: &I18n) {
    match run {
        RunView::Idle => {}
        RunView::Running {
            scanned_source,
            scanned_target,
            differences,
            cancelled,
            failure,
            ..
        } => {
            let scanned = i18n.t_with(
                "compare.run.scanned",
                &[
                    ("source", &scanned_source.to_string()),
                    ("target", &scanned_target.to_string()),
                ],
            );
            ui.add(egui::ProgressBar::new(0.0).animate(true).text(scanned));
            let found = differences.to_string();
            ui.label(i18n.t_with("compare.run.differences", &[("count", &found)]));
            if *cancelled {
                ui.label(i18n.t("compare.run.cancelling"));
            }
            show_failure(ui, failure.as_ref(), i18n);
        }
        RunView::Finished {
            cancelled, failure, ..
        } => {
            let done = if *cancelled {
                "compare.run.cancelled"
            } else {
                "compare.run.finished"
            };
            ui.label(i18n.t(done));
            show_failure(ui, failure.as_ref(), i18n);
        }
    }
}

fn show_failure(ui: &mut egui::Ui, failure: Option<&CoreError>, i18n: &I18n) {
    let Some(error) = failure else {
        return;
    };
    let color = ui.visuals().error_fg_color;
    ui.colored_label(color, i18n.t("compare.run.failed"));
    ui.colored_label(color, i18n.error(error));
}
