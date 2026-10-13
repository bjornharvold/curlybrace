//! The running and finished panels of an apply: progress per pair with Cancel, then the
//! outcome counts, per-pair rows, the failed documents (keys are data, the error text comes from
//! the typed error) and the note to run the pairs again.
use super::pending::pair_text;
use super::table::status_text;
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{
    ApplyRow, ApplySummary, ApplyUi, CompareState, StatusIcon, key_label,
};
use curlybrace_core::compare::apply::ApplyReport;
use curlybrace_core::compare::protocol::PairId;
use curlybrace_core::error::CoreError;
use eframe::egui;

/// Most failed documents listed.
const MAX_LISTED: usize = 100;
const FAILURES_HEIGHT: f32 = 240.0;
const BAR_WIDTH: f32 = 240.0;

/// The panel of the running or the finished apply; nothing otherwise.
pub fn show_progress(
    ui: &mut egui::Ui,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    match &state.apply {
        ApplyUi::Running {
            rows,
            cancel_requested,
            ..
        } => show_running(ui, state, rows, *cancel_requested, i18n),
        ApplyUi::Finished {
            reports,
            cancelled,
            requested,
        } => show_finished(ui, state, reports, (*cancelled, *requested), i18n),
        ApplyUi::Idle | ApplyUi::Reviewing(_) => None,
    }
}

fn show_running(
    ui: &mut egui::Ui,
    state: &CompareState,
    rows: &[ApplyRow],
    cancel_requested: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.horizontal(|ui| {
        ui.strong(i18n.t("compare.apply.running.title"));
        ui.spinner();
    });
    for row in rows {
        ui.horizontal(|ui| {
            ui.label(pair_text(state, row.pair, i18n));
            let fraction = match row.total {
                0 => 0.0,
                total => (row.done as f32 / total as f32).clamp(0.0, 1.0),
            };
            ui.add(egui::ProgressBar::new(fraction).desired_width(BAR_WIDTH));
            let params = [
                ("done", row.done.to_string()),
                ("total", row.total.to_string()),
            ];
            let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
            ui.label(i18n.t_with("compare.apply.running.progress", &params));
        });
    }
    let mut action = None;
    ui.horizontal(|ui| {
        let cancel = egui::Button::new(i18n.t("compare.apply.running.cancel"));
        if ui.add_enabled(state.apply.can_cancel(), cancel).clicked() {
            action = Some(CompareAction::CancelApply);
        }
        if cancel_requested {
            ui.spinner();
            ui.label(i18n.t("compare.apply.running.cancelling"));
        }
    });
    action
}

fn show_finished(
    ui: &mut egui::Ui,
    state: &CompareState,
    reports: &[(PairId, ApplyReport)],
    (cancelled, requested): (bool, u64),
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.strong(i18n.t("compare.apply.finished.title"));
    if cancelled {
        ui.label(i18n.t("compare.apply.finished.cancelled_note"));
    }
    if let Some(summary) = state.apply.summary() {
        show_summary(ui, &summary, i18n);
    }
    ui.label(i18n.t_with(
        "compare.apply.finished.requested",
        &[("count", &requested.to_string())],
    ));
    for (pair, report) in reports {
        show_pair_counts(ui, state, *pair, report, i18n);
    }
    show_failures(ui, state, reports, i18n);
    show_rerun_note(ui, i18n);
    let dismiss = ui.button(i18n.t("compare.apply.finished.dismiss"));
    dismiss
        .clicked()
        .then_some(CompareAction::DismissApplyResult)
}

/// The five outcomes, each with its icon and its label.
fn show_summary(ui: &mut egui::Ui, summary: &ApplySummary, i18n: &I18n) {
    let outcomes = [
        (
            StatusIcon::Ok,
            "compare.apply.outcome.applied",
            summary.applied,
        ),
        (
            StatusIcon::Warning,
            "compare.apply.outcome.stale",
            summary.stale,
        ),
        (
            StatusIcon::Discarded,
            "compare.apply.outcome.not_applicable",
            summary.not_planned,
        ),
        (
            StatusIcon::Failed,
            "compare.apply.outcome.failed",
            summary.failed,
        ),
        (
            StatusIcon::Cancelled,
            "compare.apply.outcome.not_run",
            summary.not_run,
        ),
    ];
    ui.horizontal_wrapped(|ui| {
        for (icon, key, count) in outcomes {
            let label = i18n.t_with(key, &[("count", &count.to_string())]);
            ui.label(status_text(icon, &label));
        }
    });
}

fn show_pair_counts(
    ui: &mut egui::Ui,
    state: &CompareState,
    pair: PairId,
    report: &ApplyReport,
    i18n: &I18n,
) {
    let counts = i18n.t_with(
        "compare.apply.finished.pair_counts",
        &[
            ("applied", &report.applied.to_string()),
            ("stale", &report.stale.to_string()),
            ("not_planned", &report.not_planned.to_string()),
            ("failed", &report.failed.to_string()),
            ("not_run", &report.not_run.to_string()),
        ],
    );
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(pair_text(state, pair, i18n)).strong());
        ui.label(counts);
    });
}

/// Up to `MAX_LISTED` failed documents, then a note when there are more, and the explanation of
/// an unknown outcome when one is among them.
fn show_failures(
    ui: &mut egui::Ui,
    state: &CompareState,
    reports: &[(PairId, ApplyReport)],
    i18n: &I18n,
) {
    let all: Vec<_> = reports
        .iter()
        .flat_map(|(pair, report)| report.failures.iter().map(move |f| (*pair, f)))
        .collect();
    if all.is_empty() && !reports.iter().any(|(_, r)| r.more_failures) {
        return;
    }
    ui.strong(i18n.t("compare.apply.finished.failures_title"));
    egui::ScrollArea::vertical()
        .id_salt("compare_apply_failures")
        .max_height(FAILURES_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, (pair, failure)) in all.iter().take(MAX_LISTED).enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(pair_text(state, *pair, i18n));
                        ui.label(egui::RichText::new(key_label(&failure.key)).monospace());
                        let color = ui.visuals().error_fg_color;
                        ui.colored_label(color, i18n.error(&failure.error));
                    });
                });
            }
        });
    let more = all.len() > MAX_LISTED || reports.iter().any(|(_, r)| r.more_failures);
    if more {
        ui.label(i18n.t("compare.apply.finished.more_failures"));
    }
    if all
        .iter()
        .any(|(_, f)| matches!(f.error, CoreError::ApplyOutcomeUnknown))
    {
        let color = ui.visuals().warn_fg_color;
        ui.colored_label(color, i18n.t("compare.apply.finished.outcome_unknown_note"));
    }
}

fn show_rerun_note(ui: &mut egui::Ui, i18n: &I18n) {
    let color = ui.visuals().warn_fg_color;
    let note = egui::RichText::new(i18n.t("compare.apply.finished.rerun_note"))
        .strong()
        .size(16.0)
        .color(color);
    ui.label(note);
}
