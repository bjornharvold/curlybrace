//! The export modal outside the options: waiting for the file dialog, the running job with its
//! progress and Cancel, and the results (what was written, skipped and left behind). There is no
//! way to close the modal while the dialog is open or a job runs, and the dialog itself is never
//! driven from here: the app layer reacts to the `Choosing` state.
use super::export::title_key;
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{ExportKind, ExportUi, RedactedPath};
use curlybrace_core::compare::protocol::ExportOutcome;
use eframe::egui;

const BAR_WIDTH: f32 = 320.0;

/// The dialog is open: only a waiting text. The dialog's answer is the only way out.
pub fn show_choosing(ui: &mut egui::Ui, kind: ExportKind, i18n: &I18n) -> Option<CompareAction> {
    ui.heading(i18n.t(title_key(kind)));
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(i18n.t("compare.export.choosing.waiting"));
    });
    ui.label(egui::RichText::new(i18n.t("compare.export.choosing.hint")).weak());
    // Back to the options; a late answer of the dialog is then dropped by the app.
    let cancel = ui.button(i18n.t("compare.export.cancel"));
    cancel
        .clicked()
        .then_some(CompareAction::ExportPathCancelled)
}

/// The running job, the result of a file export or the copied report.
pub fn show(ui: &mut egui::Ui, export: &ExportUi, i18n: &I18n) -> Option<CompareAction> {
    match export {
        ExportUi::Running {
            done,
            total,
            cancel_requested,
            ..
        } => show_running(ui, (*done, *total), *cancel_requested, i18n),
        ExportUi::Finished {
            outcome,
            kind,
            path,
        } => show_finished(ui, outcome, *kind, path.as_ref(), i18n),
        ExportUi::Copied { rows, .. } => show_copied(ui, *rows, i18n),
        ExportUi::Idle | ExportUi::Configuring(_) | ExportUi::Choosing(_) => None,
    }
}

fn show_running(
    ui: &mut egui::Ui,
    (done, total): (u64, u64),
    cancel_requested: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.horizontal(|ui| {
        ui.heading(i18n.t("compare.export.running.title"));
        ui.spinner();
    });
    let fraction = match total {
        0 => 0.0,
        total => (done as f32 / total as f32).clamp(0.0, 1.0),
    };
    ui.horizontal(|ui| {
        let bar = ui.add(egui::ProgressBar::new(fraction).desired_width(BAR_WIDTH));
        let name = i18n.t("compare.export.running.bar_name");
        bar.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, name.clone())
        });
        ui.label(progress_text(done, total, i18n));
    });
    ui.horizontal(|ui| {
        let cancel = egui::Button::new(i18n.t("compare.export.running.cancel"));
        let clicked = ui.add_enabled(!cancel_requested, cancel).clicked();
        if cancel_requested {
            ui.label(i18n.t("compare.export.running.cancelling"));
        }
        clicked.then_some(CompareAction::CancelExport)
    })
    .inner
}

/// "done of total", or only the rows done while the total is not known.
fn progress_text(done: u64, total: u64, i18n: &I18n) -> String {
    match total {
        0 => i18n.t_with(
            "compare.export.running.rows",
            &[("count", &done.to_string())],
        ),
        total => i18n.t_with(
            "compare.export.running.progress",
            &[("done", &done.to_string()), ("total", &total.to_string())],
        ),
    }
}

fn count_line(ui: &mut egui::Ui, key: &str, count: u64, i18n: &I18n) {
    ui.label(i18n.t_with(key, &[("count", &count.to_string())]));
}

fn show_finished(
    ui: &mut egui::Ui,
    outcome: &ExportOutcome,
    kind: ExportKind,
    path: Option<&RedactedPath>,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.heading(i18n.t("compare.export.finished.title"));
    if outcome.cancelled {
        ui.label(i18n.t("compare.export.finished.cancelled"));
    }
    match (kind, path) {
        // A copy that ended without a file wrote nothing: never claim rows.
        (ExportKind::Report, None) => {
            ui.label(i18n.t("compare.export.finished.nothing_copied"));
        }
        (ExportKind::Plan, _) => show_plan_counts(ui, outcome, i18n),
        (ExportKind::Report, Some(_)) => show_report_counts(ui, outcome, i18n),
    }
    if let Some(path) = path {
        show_file(ui, outcome, path, i18n);
    }
    dismiss(ui, i18n)
}

/// What an export of the pending changes did, operation by operation.
fn show_plan_counts(ui: &mut egui::Ui, outcome: &ExportOutcome, i18n: &I18n) {
    count_line(ui, "compare.export.finished.written", outcome.written, i18n);
    count_line(ui, "compare.export.finished.skipped", outcome.skipped, i18n);
    count_line(ui, "compare.export.finished.failed", outcome.failed, i18n);
    count_line(ui, "compare.export.finished.not_run", outcome.not_run, i18n);
}

/// A report counts rows: the stale and unreadable ones are written too, with an empty paths
/// cell, so they are part of the rows written, not extra.
fn show_report_counts(ui: &mut egui::Ui, outcome: &ExportOutcome, i18n: &I18n) {
    count_line(
        ui,
        "compare.export.finished.report_rows",
        outcome.written,
        i18n,
    );
    if outcome.skipped > 0 {
        count_line(
            ui,
            "compare.export.finished.report_stale",
            outcome.skipped,
            i18n,
        );
    }
    if outcome.failed > 0 {
        let key = "compare.export.finished.report_unreadable";
        count_line(ui, key, outcome.failed, i18n);
    }
    if outcome.not_run > 0 {
        count_line(ui, "compare.export.finished.not_run", outcome.not_run, i18n);
    }
}

/// The size, the path (data: selectable, shown as it is) and what became of the file.
fn show_file(ui: &mut egui::Ui, outcome: &ExportOutcome, path: &RedactedPath, i18n: &I18n) {
    count_line(ui, "compare.export.finished.bytes", outcome.bytes, i18n);
    ui.horizontal_wrapped(|ui| {
        ui.label(i18n.t("compare.export.finished.path"));
        let text = path.as_path().to_string_lossy().into_owned();
        ui.add(egui::Label::new(egui::RichText::new(text).monospace()).selectable(true));
    });
    if outcome.file_left {
        let color = ui.visuals().error_fg_color;
        let warning = egui::RichText::new(i18n.t("compare.export.finished.file_left"))
            .strong()
            .color(color);
        ui.label(warning);
    } else if outcome.partial_removed {
        ui.label(i18n.t("compare.export.finished.partial_removed"));
    }
}

fn show_copied(ui: &mut egui::Ui, rows: u64, i18n: &I18n) -> Option<CompareAction> {
    ui.heading(i18n.t("compare.export.copied.title"));
    count_line(ui, "compare.export.copied.rows", rows, i18n);
    ui.label(egui::RichText::new(i18n.t("compare.export.copied.note")).weak());
    dismiss(ui, i18n)
}

fn dismiss(ui: &mut egui::Ui, i18n: &I18n) -> Option<CompareAction> {
    ui.separator();
    let button = ui.button(i18n.t("compare.export.dismiss"));
    button.clicked().then_some(CompareAction::CloseExport)
}
