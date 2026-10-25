//! The progress of a copy (a bar per collection, Cancel at all times) and, once it ended, its
//! report: exactly what was written per collection, why it stopped, and the documents that were
//! refused by `_id` (bounded, never values). Every reason is a message key.
use crate::I18n;
use crate::app::copy::CopyInput;
use crate::app::copy_state::{CopyRow, CopyRun, RunOutcome};
use eframe::egui;

const MODAL_WIDTH: f32 = 620.0;
const BAR_WIDTH: f32 = 360.0;
const FAILURES_HEIGHT: f32 = 220.0;

/// The progress while the copy runs, the report after it ended.
pub fn show_progress(ctx: &egui::Context, run: &CopyRun, i18n: &I18n) -> Option<CopyInput> {
    let modal = egui::Modal::new(egui::Id::new("copy_progress"));
    let response = modal.show(ctx, |ui| {
        ui.set_width(MODAL_WIDTH);
        match &run.outcome {
            None => show_running(ui, run, i18n),
            Some(outcome) => show_report(ui, run, outcome, i18n),
        }
    });
    // A click on the backdrop must never stop a copy that is under way: only the Cancel button
    // and Escape do. The finished report closes either way.
    let backdrop = response.backdrop_response.clicked();
    let closed = response.should_close();
    let leave = match run.is_finished() {
        true => closed.then_some(CopyInput::Close),
        false => (closed && !backdrop).then_some(CopyInput::Cancel),
    };
    response.inner.or(leave)
}

fn show_running(ui: &mut egui::Ui, run: &CopyRun, i18n: &I18n) -> Option<CopyInput> {
    ui.horizontal(|ui| {
        ui.heading(i18n.t("copy.progress.title"));
        ui.spinner();
    });
    for row in &run.rows {
        show_row(ui, row, i18n);
    }
    ui.separator();
    let cancel = ui.add_enabled(
        run.can_cancel(),
        egui::Button::new(i18n.t("copy.progress.cancel")),
    );
    if run.cancel_requested {
        ui.label(i18n.t("copy.progress.cancelling"));
    }
    cancel.clicked().then_some(CopyInput::Cancel)
}

fn show_row(ui: &mut egui::Ui, row: &CopyRow, i18n: &I18n) {
    let collection = row.target.collection.as_str();
    if !row.started() {
        ui.label(i18n.t_with("copy.progress.row_waiting", &[("collection", collection)]));
        return;
    }
    let p = row.progress;
    let mut params = vec![
        ("collection", collection.to_string()),
        ("written", p.written.to_string()),
        ("skipped", p.skipped.to_string()),
        ("failed", p.failed.to_string()),
    ];
    let (key, fraction) = match (row.finished, row.estimated_total) {
        (false, Some(total)) if total > 0 => {
            params.push(("scanned", p.scanned.to_string()));
            params.push(("total", total.to_string()));
            (
                "copy.progress.row_estimated",
                p.scanned as f32 / total as f32,
            )
        }
        (finished, _) => ("copy.progress.row", if finished { 1.0 } else { 0.0 }),
    };
    let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let text = i18n.t_with(key, &params);
    ui.add(
        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
            .desired_width(BAR_WIDTH)
            .text(text),
    );
}

fn show_report(
    ui: &mut egui::Ui,
    run: &CopyRun,
    outcome: &RunOutcome,
    i18n: &I18n,
) -> Option<CopyInput> {
    let title = match outcome {
        RunOutcome::Completed => "copy.report.title_completed",
        RunOutcome::Cancelled => "copy.report.title_cancelled",
        RunOutcome::Failed(_) => "copy.report.title_failed",
    };
    ui.heading(i18n.t(title));
    show_reason(ui, run, outcome, i18n);
    show_written(ui, run, i18n);
    show_structure(ui, run, i18n);
    if !matches!(outcome, RunOutcome::Completed) {
        ui.label(i18n.t("copy.report.nothing_else"));
    }
    if run.counts_incomplete {
        ui.weak(i18n.t("copy.report.counts_incomplete"));
    }
    show_failures(ui, run, i18n);
    ui.separator();
    let close = ui.button(i18n.t("copy.report.close"));
    if !close.has_focus() {
        close.request_focus();
    }
    close.clicked().then_some(CopyInput::Close)
}

/// Why it stopped: a lost connection says so, any other error is its localized message.
fn show_reason(ui: &mut egui::Ui, run: &CopyRun, outcome: &RunOutcome, i18n: &I18n) {
    let RunOutcome::Failed(error) = outcome else {
        return;
    };
    let reason = match run.session_lost() {
        true => i18n.t("copy.report.session_lost"),
        false => i18n.t_with("copy.report.error", &[("reason", &i18n.error(error))]),
    };
    ui.colored_label(ui.visuals().error_fg_color, reason);
}

/// Exactly what was written, per collection.
fn show_written(ui: &mut egui::Ui, run: &CopyRun, i18n: &I18n) {
    if run.rows.is_empty() || run.nothing_written() {
        ui.label(i18n.t("copy.report.nothing_written"));
    }
    for (collection, written) in run.written_lines() {
        let text = match written {
            Some(count) => i18n.t_with(
                "copy.report.written",
                &[("collection", &collection), ("count", &count.to_string())],
            ),
            None => i18n.t_with("copy.report.not_started", &[("collection", &collection)]),
        };
        ui.label(text);
    }
    for line in run.not_written_lines() {
        let params = [
            ("collection", line.collection),
            ("skipped", line.skipped.to_string()),
            ("failed", line.failed.to_string()),
        ];
        let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        ui.label(i18n.t_with("copy.report.not_written", &params));
    }
}

/// The options and indexes of each collection: created with the source's options or kept as
/// they were, the index counts, and every index that failed by its name and reason.
fn show_structure(ui: &mut egui::Ui, run: &CopyRun, i18n: &I18n) {
    for line in run.options_lines() {
        ui.label(i18n.t_with(line.key, &[("collection", &line.collection)]));
    }
    for line in run.index_lines() {
        let params = [
            ("collection", line.collection),
            ("built", line.built.to_string()),
            ("existing", line.existing.to_string()),
            ("failed", line.failed.to_string()),
        ];
        let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        ui.label(i18n.t_with("copy.report.indexes", &params));
    }
    for row in run.index_failures() {
        let reason = i18n.t(row.reason_key);
        let params = [
            ("collection", row.collection.as_str()),
            ("name", row.name.as_str()),
            ("reason", reason.as_str()),
        ];
        let text = i18n.t_with("copy.report.index_failed", &params);
        ui.colored_label(ui.visuals().warn_fg_color, text);
    }
    if run.indexes_skipped() {
        ui.label(i18n.t("copy.report.indexes_not_built"));
    }
}

fn show_failures(ui: &mut egui::Ui, run: &CopyRun, i18n: &I18n) {
    let rows = run.failure_rows();
    if rows.is_empty() && !run.more_failures {
        return;
    }
    ui.separator();
    ui.strong(i18n.t("copy.report.failures_title"));
    egui::ScrollArea::vertical()
        .id_salt("copy_failures")
        .max_height(FAILURES_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, row) in rows.iter().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(&row.collection);
                        ui.label(egui::RichText::new(&row.key).monospace());
                        ui.colored_label(ui.visuals().error_fg_color, i18n.t(row.reason_key));
                    });
                });
            }
        });
    if run.more_failures {
        let params = [
            ("listed", rows.len().to_string()),
            ("failed", run.failed_total().to_string()),
        ];
        let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        ui.label(i18n.t_with("copy.report.more_failures", &params));
    }
}
