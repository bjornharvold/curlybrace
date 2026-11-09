//! The undo parts of the bulk dialog: the note about the undo copy before the write, what the
//! report says about undoing, and the undo's own screens (preview with the typed confirmation,
//! running, report). Each draws the flow it is given and reports the edit as a [`BulkAction`].
use super::bulk_dialog::describe;
use super::index_dialogs::focus_once;
use super::tone_notice::tone_notice;
use super::typed_confirmation::{TypedConfirmation, show_typed_confirmation};
use crate::I18n;
use crate::app::BulkAction;
use crate::theme::{Tone, filled_button, gate, text_button};
use crate::viewmodel::{BulkFlow, UndoDocText, UndoFlow, UndoPreviewState, UndoStep};
use curlybrace_core::bulk::{UndoPolicy, UndoStatus};
use curlybrace_core::snapshot::UndoReport;
use eframe::egui;

const SAMPLES_MAX_HEIGHT: f32 = 360.0;

/// Where the undo copy is kept, as said to the person; a fallback wording when the OS gives no
/// settings folder.
fn snapshot_folder(flow: &BulkFlow, i18n: &I18n) -> String {
    flow.undo_folder
        .clone()
        .unwrap_or_else(|| i18n.t("bulk.undo.folder_unknown"))
}

/// The line before the write: that an undo copy is saved and where, or, after the person gave
/// it up, that the write cannot be undone.
pub(super) fn undo_notice(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    match flow.undo_policy {
        UndoPolicy::None => tone_notice(ui, Tone::Bad, &i18n.t("bulk.cannot_undo"), true),
        UndoPolicy::Snapshot => {
            let folder = snapshot_folder(flow, i18n);
            let text = i18n.t_with("bulk.undo.copy_notice", &[("folder", &folder)]);
            tone_notice(ui, Tone::Neutral, &text, false);
        }
    }
}

/// What the report of the write says about undoing it.
pub(super) fn report_undo_status(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    if flow.snapshot_refused() {
        tone_notice(
            ui,
            Tone::Warning,
            &i18n.t("bulk.undo.snapshot_refused"),
            false,
        );
        return;
    }
    match flow.undo {
        UndoStatus::NotRequested | UndoStatus::NothingToUndo => {}
        UndoStatus::Lost => tone_notice(ui, Tone::Bad, &i18n.t("bulk.undo.lost"), true),
        UndoStatus::Uncovered(n) => {
            let text = i18n.t_with("bulk.undo.uncovered_only", &[("n", &n.to_string())]);
            tone_notice(ui, Tone::Warning, &text, true);
        }
        UndoStatus::Offered(offer) => {
            let n = offer.documents.to_string();
            ui.label(i18n.t_with("bulk.undo.available", &[("n", &n)]));
            if offer.uncovered > 0 {
                let more = offer.uncovered.to_string();
                let text = i18n.t_with("bulk.undo.uncovered", &[("n", &more)]);
                tone_notice(ui, Tone::Warning, &text, false);
            }
        }
    }
}

/// The footer of the write's report: close (keeping or deleting the undo copy), start the undo,
/// or run without undo after the copy was refused.
pub(super) fn report_footer(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction> {
    let offered = matches!(flow.undo, UndoStatus::Offered(_));
    let close_label = match offered {
        true => "bulk.undo.keep_copy",
        false => "common.close",
    };
    let mut action = None;
    let close = text_button(ui, i18n.t(close_label));
    focus_once(ui, &close, ("bulk_focus", "report"));
    if close.clicked() {
        action = Some(BulkAction::Close);
    }
    if offered {
        if text_button(ui, i18n.t("bulk.undo.discard_copy")).clicked() {
            action = Some(BulkAction::DiscardCopy);
        }
        if filled_button(ui, i18n.t("bulk.undo.open")).clicked() {
            action = Some(BulkAction::OpenUndo);
        }
    }
    if flow.snapshot_refused() && filled_button(ui, i18n.t("bulk.undo.run_without")).clicked() {
        action = Some(BulkAction::RunWithoutUndo);
    }
    action
}

// ------------------------------------------------------------------ the undo's own screens

pub(super) fn undo_body(
    ui: &mut egui::Ui,
    flow: &BulkFlow,
    undo: &UndoFlow,
    connection: &str,
    i18n: &I18n,
) -> Option<BulkAction> {
    match undo.step {
        UndoStep::Confirm => confirm_body(ui, flow, undo, connection, i18n),
        UndoStep::Running => {
            ui.label(i18n.t("bulk.undo.running"));
            ui.label(i18n.t("bulk.undo.running_note"));
            None
        }
        UndoStep::Report => {
            report_body(ui, undo, i18n);
            None
        }
    }
}

fn confirm_body(
    ui: &mut egui::Ui,
    flow: &BulkFlow,
    undo: &UndoFlow,
    connection: &str,
    i18n: &I18n,
) -> Option<BulkAction> {
    if flow.production {
        let text = i18n.t_with("bulk.production_notice", &[("connection", connection)]);
        tone_notice(ui, Tone::Bad, &text, true);
    }
    match &undo.preview {
        UndoPreviewState::Pending => {
            ui.label(i18n.t("bulk.undo.preview_pending"));
            None
        }
        UndoPreviewState::Failed(error) => {
            tone_notice(ui, Tone::Bad, &i18n.t("bulk.undo.preview_failed"), true);
            ui.label(i18n.t(error.i18n_key()));
            None
        }
        UndoPreviewState::Ready(preview) => {
            let n = preview.documents.to_string();
            ui.label(i18n.t_with("bulk.undo.intro", &[("n", &n)]));
            ui.label(egui::RichText::new(i18n.t("bulk.undo.preview_heading")).strong());
            samples(ui, undo, i18n);
            typed_field(ui, undo, i18n)
        }
    }
}

fn samples(ui: &mut egui::Ui, undo: &UndoFlow, i18n: &I18n) {
    egui::ScrollArea::vertical()
        .id_salt("bulk_undo_samples")
        .max_height(SAMPLES_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for doc in undo.preview_docs().iter() {
                sample(ui, doc, i18n);
            }
        });
}

fn sample(ui: &mut egui::Ui, doc: &UndoDocText, i18n: &I18n) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        caption(ui, i18n.t("bulk.undo.now"));
        match &doc.current {
            Some(text) => mono(ui, text),
            None => {
                ui.label(i18n.t("bulk.undo.gone"));
            }
        }
        caption(ui, i18n.t("bulk.undo.restore_to"));
        mono(ui, &doc.original);
    });
}

fn caption(ui: &mut egui::Ui, text: String) {
    ui.label(egui::RichText::new(text).strong());
}

fn mono(ui: &mut egui::Ui, text: &str) {
    let text = egui::RichText::new(text).monospace();
    ui.add(egui::Label::new(text).wrap().selectable(true));
}

/// Undo is a write: it always asks for the collection name, whatever the size.
fn typed_field(ui: &mut egui::Ui, undo: &UndoFlow, i18n: &I18n) -> Option<BulkAction> {
    let name = undo.collection.as_str();
    let caption = i18n.t_with("bulk.type_collection_name", &[("name", name)]);
    let prompt = TypedConfirmation {
        instruction: &i18n.t("bulk.type_instruction"),
        required: name,
        field_label: &i18n.t("bulk.type_field"),
        accessible_name: &caption,
    };
    let mut typed = undo.typed.clone();
    let field = show_typed_confirmation(ui, &prompt, &mut typed);
    if !undo.can_run() {
        ui.label(i18n.t("bulk.run_blocked"));
    }
    field.changed().then_some(BulkAction::SetUndoTyped(typed))
}

fn report_body(ui: &mut egui::Ui, undo: &UndoFlow, i18n: &I18n) {
    match &undo.report {
        Some(Ok(report)) => counts(ui, report, i18n),
        Some(Err(error)) => {
            tone_notice(ui, Tone::Bad, &i18n.t("bulk.undo.failed"), true);
            ui.label(i18n.t(error.i18n_key()));
        }
        None => {}
    }
}

fn counts(ui: &mut egui::Ui, report: &UndoReport, i18n: &I18n) {
    let lines = [
        ("bulk.undo.report.restored", report.restored),
        ("bulk.undo.report.skipped_changed", report.skipped_changed),
        ("bulk.undo.report.skipped_present", report.skipped_present),
        ("bulk.undo.report.unchanged", report.unchanged),
        ("bulk.undo.report.errors", report.errors),
    ];
    for (key, n) in lines {
        if n > 0 || key == "bulk.undo.report.restored" {
            ui.label(i18n.t_with(key, &[("n", &n.to_string())]));
        }
    }
}

/// Whether the report is one worth retrying: the undo did not finish, or some documents failed.
fn retryable(undo: &UndoFlow) -> bool {
    match &undo.report {
        Some(Err(_)) => true,
        Some(Ok(report)) => report.errors > 0,
        None => false,
    }
}

pub(super) fn undo_footer(ui: &mut egui::Ui, undo: &UndoFlow, i18n: &I18n) -> Option<BulkAction> {
    match undo.step {
        UndoStep::Confirm => {
            let mut action = None;
            let back = text_button(ui, i18n.t("bulk.undo.back"));
            focus_once(ui, &back, ("bulk_focus", "undo_confirm"));
            if back.clicked() {
                action = Some(BulkAction::CloseUndo);
            }
            let run = gate(ui, undo.can_run(), |ui| {
                filled_button(ui, i18n.t("bulk.undo.run"))
            });
            if run.clicked() {
                action = Some(BulkAction::RunUndo);
            }
            action
        }
        // The undo is not stopped halfway: documents already restored stay restored.
        UndoStep::Running => None,
        UndoStep::Report => {
            let mut action = None;
            let close = text_button(ui, i18n.t("common.close"));
            focus_once(ui, &close, ("bulk_focus", "undo_report"));
            describe(ui, &close, &i18n.t("bulk.undo.keep_copy"));
            if close.clicked() {
                action = Some(BulkAction::Close);
            }
            if retryable(undo) && filled_button(ui, i18n.t("bulk.undo.retry")).clicked() {
                action = Some(BulkAction::RetryUndo);
            }
            action
        }
    }
}
