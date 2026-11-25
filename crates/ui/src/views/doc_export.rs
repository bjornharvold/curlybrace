//! The document export modal: the options, the wait for the file dialog, the running job with its
//! progress and Cancel, and the result. One `egui::Modal` for every state. The views only report
//! actions: the file dialog is opened by the app layer from the `Choosing` state, and nothing here
//! starts a write. Escape or a click outside closes the modal only while the options are shown.
use crate::I18n;
use crate::app::doc_export::DocExportAction;
use crate::theme::typography::dialog_title;
use crate::theme::{filled_button, gate, modal_frame, text_button};
use crate::viewmodel::compare::RedactedPath;
use crate::viewmodel::doc_export::{DocExportDraft, DocExportSource, DocExportUi};
use curlybrace_core::error::CoreError;
use curlybrace_core::export::protocol::{DocExportOutcome, DocFormat, JsonMode};
use eframe::egui;

const MODAL_WIDTH: f32 = 520.0;
const BAR_WIDTH: f32 = 320.0;

/// The modal, while an export is open; `None` when idle.
pub fn show_doc_export(
    ctx: &egui::Context,
    export: &DocExportUi,
    i18n: &I18n,
) -> Option<DocExportAction> {
    if *export == DocExportUi::Idle {
        return None;
    }
    let modal = egui::Modal::new(egui::Id::new("doc_export")).frame(modal_frame(ctx));
    let response = modal.show(ctx, |ui| show_body(ui, export, i18n));
    let configuring = matches!(export, DocExportUi::Configuring { .. });
    let closes = response.should_close() && configuring;
    response.inner.or(closes.then_some(DocExportAction::Close))
}

fn show_body(ui: &mut egui::Ui, export: &DocExportUi, i18n: &I18n) -> Option<DocExportAction> {
    ui.set_width(MODAL_WIDTH);
    match export {
        DocExportUi::Configuring { source, draft } => show_options(ui, source, *draft, i18n),
        DocExportUi::Choosing { .. } => show_choosing(ui, i18n),
        DocExportUi::Running {
            total,
            done,
            cancel_requested,
            ..
        } => show_running(ui, (*done, *total), *cancel_requested, i18n),
        DocExportUi::Finished { outcome, path } => show_finished(ui, outcome, path.as_ref(), i18n),
        DocExportUi::Idle => None,
    }
}

fn show_options(
    ui: &mut egui::Ui,
    source: &DocExportSource,
    draft: DocExportDraft,
    i18n: &I18n,
) -> Option<DocExportAction> {
    ui.label(dialog_title(ui, i18n.t("export.title")));
    show_source(ui, source, i18n);
    let format = show_format(ui, draft, i18n);
    let mode = show_mode(ui, draft, i18n);
    show_notes(ui, i18n);
    let buttons = show_buttons(ui, i18n);
    buttons.or(mode).or(format)
}

fn show_source(ui: &mut egui::Ui, source: &DocExportSource, i18n: &I18n) {
    let name = format!("{}.{}", source.ns.db, source.ns.collection);
    let line = match source.filter {
        Some(_) => i18n.t_with("export.source.query", &[("name", &name)]),
        None => i18n.t_with("export.source.collection", &[("name", &name)]),
    };
    ui.label(line);
    if source.filter.is_some() && source.ignores_options {
        ui.label(egui::RichText::new(i18n.t("export.source.query_ignored")).weak());
    }
}

fn show_format(ui: &mut egui::Ui, draft: DocExportDraft, i18n: &I18n) -> Option<DocExportAction> {
    ui.separator();
    ui.strong(i18n.t("export.format.title"));
    let mut action = None;
    for (format, label) in [
        (DocFormat::Ndjson, i18n.t("export.format.ndjson")),
        (DocFormat::JsonArray, i18n.t("export.format.json_array")),
    ] {
        let chosen = draft.format == format;
        if ui.add(egui::RadioButton::new(chosen, label)).clicked() && !chosen {
            action = Some(DocExportAction::Edit(DocExportDraft { format, ..draft }));
        }
    }
    action
}

fn show_mode(ui: &mut egui::Ui, draft: DocExportDraft, i18n: &I18n) -> Option<DocExportAction> {
    ui.separator();
    ui.strong(i18n.t("export.mode.title"));
    let mut action = None;
    for (mode, label, hint) in [
        (
            JsonMode::Canonical,
            i18n.t("export.mode.canonical"),
            i18n.t("export.mode.canonical_hint"),
        ),
        (
            JsonMode::Relaxed,
            i18n.t("export.mode.relaxed"),
            i18n.t("export.mode.relaxed_hint"),
        ),
    ] {
        let chosen = draft.mode == mode;
        if ui.add(egui::RadioButton::new(chosen, label)).clicked() && !chosen {
            action = Some(DocExportAction::Edit(DocExportDraft { mode, ..draft }));
        }
        ui.indent("export_mode_hint", |ui| {
            ui.label(egui::RichText::new(hint).weak());
        });
    }
    action
}

fn show_notes(ui: &mut egui::Ui, i18n: &I18n) {
    ui.separator();
    let color = ui.visuals().warn_fg_color;
    ui.colored_label(color, i18n.t("export.note.sensitive"));
    ui.label(egui::RichText::new(i18n.t("export.note.consistency")).weak());
}

/// Choose file (the way on) and Close.
fn show_buttons(ui: &mut egui::Ui, i18n: &I18n) -> Option<DocExportAction> {
    ui.separator();
    ui.horizontal(|ui| {
        let mut action = None;
        if filled_button(ui, i18n.t("export.choose_file")).clicked() {
            action = Some(DocExportAction::ChooseFile);
        }
        if text_button(ui, i18n.t("export.finished.close")).clicked() {
            action = Some(DocExportAction::Close);
        }
        action
    })
    .inner
}

/// The dialog is open: only a waiting text and the way back.
fn show_choosing(ui: &mut egui::Ui, i18n: &I18n) -> Option<DocExportAction> {
    ui.label(dialog_title(ui, i18n.t("export.title")));
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(i18n.t("export.choosing.waiting"));
    });
    text_button(ui, i18n.t("export.choosing.cancel"))
        .clicked()
        .then_some(DocExportAction::PathCancelled)
}

fn show_running(
    ui: &mut egui::Ui,
    (done, total): (u64, Option<u64>),
    cancel_requested: bool,
    i18n: &I18n,
) -> Option<DocExportAction> {
    ui.horizontal(|ui| {
        ui.label(dialog_title(ui, i18n.t("export.running.title")));
        ui.spinner();
    });
    let fraction = match total {
        Some(total) if total > 0 && done <= total => (done as f32 / total as f32).clamp(0.0, 1.0),
        _ => 0.0,
    };
    ui.horizontal(|ui| {
        let bar = ui.add(egui::ProgressBar::new(fraction).desired_width(BAR_WIDTH));
        let name = i18n.t("export.running.bar_name");
        bar.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, name.clone())
        });
        ui.label(progress_text(done, total, i18n));
    });
    ui.horizontal(|ui| {
        let cancel = gate(ui, !cancel_requested, |ui| {
            text_button(ui, i18n.t("export.running.cancel"))
        });
        let clicked = cancel.clicked();
        if cancel_requested {
            ui.label(i18n.t("export.running.cancelling"));
        }
        clicked.then_some(DocExportAction::Cancel)
    })
    .inner
}

/// "done of total", or only the documents done while the total is not known or has been passed.
/// The noun follows the number it counts: the total, or the done count without one.
fn progress_text(done: u64, total: Option<u64>, i18n: &I18n) -> String {
    let shown = done.to_string();
    match total.filter(|t| done <= *t) {
        Some(total) => {
            let key = match total {
                1 => "export.running.progress_one",
                _ => "export.running.progress_many",
            };
            i18n.t_with(key, &[("done", &shown), ("total", &total.to_string())])
        }
        None => {
            let key = match done {
                1 => "export.running.progress_unknown_one",
                _ => "export.running.progress_unknown_many",
            };
            i18n.t_with(key, &[("done", &shown)])
        }
    }
}

/// "Exported N documents.", with the singular for one.
fn done_text(written: u64, i18n: &I18n) -> String {
    let key = match written {
        1 => "export.finished.done_one",
        _ => "export.finished.done_many",
    };
    i18n.t_with(key, &[("count", &written.to_string())])
}

fn show_finished(
    ui: &mut egui::Ui,
    outcome: &Result<DocExportOutcome, CoreError>,
    path: Option<&RedactedPath>,
    i18n: &I18n,
) -> Option<DocExportAction> {
    ui.label(dialog_title(ui, i18n.t("export.title")));
    match outcome {
        Ok(outcome) => show_outcome(ui, outcome, path, i18n),
        Err(_) => {
            ui.label(i18n.t("export.finished.failed"));
        }
    }
    ui.separator();
    filled_button(ui, i18n.t("export.finished.close"))
        .clicked()
        .then_some(DocExportAction::Close)
}

fn show_outcome(
    ui: &mut egui::Ui,
    outcome: &DocExportOutcome,
    path: Option<&RedactedPath>,
    i18n: &I18n,
) {
    let removed = outcome.cancelled && !outcome.file_left;
    match (outcome.cancelled, outcome.file_left) {
        (true, false) => ui.label(i18n.t("export.finished.cancelled")),
        (true, true) => ui.label(i18n.t("export.finished.cancelled_kept")),
        (false, _) => ui.label(done_text(outcome.written, i18n)),
    };
    // A removed file has no path worth showing.
    let path = path.filter(|_| !removed);
    if let Some(path) = path {
        // The path is data the person chose: shown as it is, selectable, never logged.
        let text = path.as_path().to_string_lossy().into_owned();
        ui.add(egui::Label::new(egui::RichText::new(text).monospace()).selectable(true));
    }
    if outcome.file_left {
        let color = ui.visuals().error_fg_color;
        let warning = egui::RichText::new(i18n.t("export.finished.file_left"))
            .strong()
            .color(color);
        ui.label(warning);
    }
}
