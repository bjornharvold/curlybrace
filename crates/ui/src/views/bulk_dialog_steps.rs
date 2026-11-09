//! The bodies of the bulk dialog's steps. Each draws the flow it is given; the ones with
//! inputs report the edit as a [`BulkAction`].
use super::bulk_undo_steps::{report_undo_status, undo_notice};
use super::tone_notice::tone_notice;
use super::typed_confirmation::{TypedConfirmation, show_typed_confirmation};
use crate::I18n;
use crate::app::BulkAction;
use crate::theme::{FieldOptions, Tone, outlined_field_opts};
use crate::viewmodel::{BulkFlow, BulkKind, CancelNote, CountState, DocText, PreviewState};
use curlybrace_core::bulk::{BulkOutcome, PreviewDoc, PreviewGap, UpdatePreview};
use curlybrace_core::error::CoreError;
use eframe::egui;

const UPDATE_ROWS: usize = 6;
const FILTER_MAX_HEIGHT: f32 = 120.0;
const SAMPLES_MAX_HEIGHT: f32 = 320.0;

pub(super) fn define_body(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction> {
    let collection = flow.ns.collection.as_str();
    let action = match flow.kind {
        BulkKind::Delete => {
            let key = "bulk.define.delete_statement";
            ui.label(i18n.t_with(key, &[("collection", collection)]));
            None
        }
        BulkKind::Update => {
            let key = "bulk.define.update_intro";
            ui.label(i18n.t_with(key, &[("collection", collection)]));
            update_inputs(ui, flow, i18n)
        }
    };
    filter_block(ui, flow, i18n);
    action
}

fn update_inputs(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction> {
    let mut action = None;
    let mut text = flow.update_text.clone();
    let message = flow.update_problem().map(|key| i18n.t(key));
    let options = FieldOptions {
        error: message.as_deref(),
        multiline_rows: Some(UPDATE_ROWS),
        reserve_error_row: true,
        ..FieldOptions::default()
    };
    let field = outlined_field_opts(
        ui,
        &i18n.t("bulk.define.update_field"),
        &mut text,
        &i18n.t("bulk.define.update_hint"),
        options,
    );
    if field.changed() {
        action = Some(BulkAction::SetUpdateText(text));
    }
    let mut upsert = flow.upsert;
    if ui
        .checkbox(&mut upsert, i18n.t("bulk.define.upsert"))
        .changed()
    {
        action = Some(BulkAction::SetUpsert(upsert));
    }
    action
}

/// The filter the write will use, read-only, and the line saying it is not just this page.
fn filter_block(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    ui.label(egui::RichText::new(i18n.t("bulk.filter_heading")).strong());
    egui::ScrollArea::vertical()
        .id_salt("bulk_filter")
        .max_height(FILTER_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| mono(ui, &flow.filter_text));
    ui.label(i18n.t("bulk.filter_scope"));
}

fn mono(ui: &mut egui::Ui, text: &str) {
    let text = egui::RichText::new(text).monospace();
    ui.add(egui::Label::new(text).wrap().selectable(true));
}

fn count_line(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    let text = match flow.count {
        CountState::Pending => i18n.t("bulk.count_pending"),
        CountState::Known(n) => i18n.t_with("bulk.count_known", &[("n", &n.to_string())]),
        CountState::Unavailable => i18n.t("bulk.count_unavailable"),
    };
    ui.label(egui::RichText::new(text).strong());
}

pub(super) fn preview_body(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    count_line(ui, flow, i18n);
    match &flow.preview {
        PreviewState::Pending => {
            ui.label(i18n.t("bulk.preview_pending"));
        }
        PreviewState::Failed(error) => failed_preview(ui, error, i18n),
        PreviewState::Update(preview) => update_preview(ui, flow, preview, i18n),
        PreviewState::Sample(_) => sample_preview(ui, flow, i18n),
    }
    undo_notice(ui, flow, i18n);
}

/// A preview that errored: the update proved invalid on the matching documents (no run is
/// offered), or the preview is unavailable (the typed-confirmed run still is). The error is told
/// by its i18n key only, never by server text.
fn failed_preview(ui: &mut egui::Ui, error: &CoreError, i18n: &I18n) {
    if *error == CoreError::BulkInvalidUpdate {
        let text = i18n.t("bulk.preview_update_invalid");
        tone_notice(ui, Tone::Bad, &text, true);
    } else {
        tone_notice(ui, Tone::Warning, &i18n.t("bulk.preview_failed"), false);
    }
    ui.label(i18n.t(error.i18n_key()));
}

fn gap_key(gap: PreviewGap) -> &'static str {
    match gap {
        PreviewGap::TransactionsUnavailable => "bulk.gap_transactions",
        PreviewGap::Conflict => "bulk.gap_conflict",
        PreviewGap::NoMatch => "bulk.gap_no_match",
    }
}

fn update_preview(ui: &mut egui::Ui, flow: &BulkFlow, preview: &UpdatePreview, i18n: &I18n) {
    if let Some(gap) = preview.gap {
        tone_notice(ui, Tone::Warning, &i18n.t(gap_key(gap)), false);
    }
    let texts = flow.preview_docs();
    samples_area(ui, |ui| {
        for (doc, text) in preview.docs.iter().zip(texts.iter()) {
            preview_doc(ui, doc, text, flow.upsert, i18n);
        }
    });
}

fn samples_area(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical()
        .id_salt("bulk_samples")
        .max_height(SAMPLES_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, add);
}

/// One sampled document: what it is now and what the update makes of it, stacked. A document
/// that is not there yet (an upsert) says so instead of showing an empty "before".
fn preview_doc(ui: &mut egui::Ui, doc: &PreviewDoc, text: &DocText, upsert: bool, i18n: &I18n) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if doc.before.is_empty() && upsert {
            ui.label(i18n.t("bulk.preview.upserted_hint"));
        } else {
            caption(ui, i18n.t("bulk.preview.before"));
            mono(ui, &text.before);
        }
        match &text.after {
            Some(after) => {
                caption(ui, i18n.t("bulk.preview.after"));
                mono(ui, after);
            }
            None => {
                ui.label(i18n.t("bulk.no_after_state"));
            }
        }
    });
}

fn caption(ui: &mut egui::Ui, text: String) {
    ui.label(egui::RichText::new(text).strong());
}

fn sample_preview(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    caption(ui, i18n.t("bulk.preview.removed_heading"));
    let texts = flow.preview_docs();
    if texts.is_empty() {
        ui.label(i18n.t("bulk.no_samples"));
        return;
    }
    samples_area(ui, |ui| {
        for text in texts.iter() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                mono(ui, &text.before);
            });
        }
    });
}

pub(super) fn confirm_body(
    ui: &mut egui::Ui,
    flow: &BulkFlow,
    connection: &str,
    i18n: &I18n,
) -> Option<BulkAction> {
    let collection = flow.ns.collection.as_str();
    let mut action = None;
    if flow.production {
        let text = i18n.t_with("bulk.production_notice", &[("connection", connection)]);
        tone_notice(ui, Tone::Bad, &text, true);
    }
    if flow.kind == BulkKind::Delete {
        let key = "bulk.define.delete_statement";
        ui.label(i18n.t_with(key, &[("collection", collection)]));
    }
    count_line(ui, flow, i18n);
    if flow.needs_all_documents_step() {
        action = all_documents(ui, flow, i18n);
    }
    if flow.needs_typed() {
        action = typed_field(ui, flow, i18n).or(action);
    }
    undo_notice(ui, flow, i18n);
    if !flow.can_run() {
        ui.label(i18n.t("bulk.run_blocked"));
    }
    action
}

fn all_documents(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction> {
    let params = [("collection", flow.ns.collection.as_str())];
    let warning = i18n.t_with("bulk.confirm.all_documents_warning", &params);
    tone_notice(ui, Tone::Bad, &warning, true);
    let mut ticked = flow.all_documents_ticked;
    let tick = i18n.t_with("bulk.confirm.all_documents_tick", &params);
    let changed = ui.checkbox(&mut ticked, tick).changed();
    changed.then_some(BulkAction::SetAllDocuments(ticked))
}

fn typed_field(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction> {
    let name = flow.ns.collection.as_str();
    let caption = i18n.t_with("bulk.type_collection_name", &[("name", name)]);
    let prompt = TypedConfirmation {
        instruction: &i18n.t("bulk.type_instruction"),
        required: name,
        field_label: &i18n.t("bulk.type_field"),
        accessible_name: &caption,
    };
    let mut typed = flow.typed.clone();
    let field = show_typed_confirmation(ui, &prompt, &mut typed);
    field.changed().then_some(BulkAction::SetTyped(typed))
}

pub(super) fn running_body(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    ui.label(i18n.t("bulk.running"));
    let note = match flow.cancel_note {
        CancelNote::None => None,
        CancelNote::Unavailable => Some("bulk.cancel_unavailable"),
        CancelNote::NothingToStop => Some("bulk.cancel_nothing_to_stop"),
        CancelNote::Requested => Some("bulk.cancel_requested"),
    };
    if let Some(key) = note {
        tone_notice(ui, Tone::Warning, &i18n.t(key), false);
    }
}

pub(super) fn report_body(ui: &mut egui::Ui, flow: &BulkFlow, i18n: &I18n) {
    match &flow.report {
        Some(Ok(BulkOutcome::Applied {
            matched,
            modified,
            upserted,
            deleted,
        })) => count_labels(
            ui,
            flow.kind,
            i18n,
            [*matched, *modified, upserted.len() as u64, *deleted],
        ),
        // Applied, but not confirmed durable: a warning (not the red of a rejection) over the
        // counts, which are known.
        Some(Ok(BulkOutcome::Unconfirmed {
            matched,
            modified,
            upserted,
            deleted,
        })) => {
            tone_notice(ui, Tone::Warning, &i18n.t("bulk.report_unconfirmed"), true);
            count_labels(
                ui,
                flow.kind,
                i18n,
                [*matched, *modified, upserted.len() as u64, *deleted],
            );
        }
        Some(Ok(BulkOutcome::Unknown { cancelled })) => {
            let key = match cancelled {
                true => "bulk.report_unknown_cancelled",
                false => "bulk.report_unknown",
            };
            tone_notice(ui, Tone::Warning, &i18n.t(key), true);
        }
        Some(Ok(BulkOutcome::Rejected { reason })) => {
            let reason = i18n.t(reason.i18n_key());
            let text = i18n.t_with("bulk.report_rejected", &[("reason", &reason)]);
            tone_notice(ui, Tone::Bad, &text, true);
        }
        Some(Err(error)) => {
            tone_notice(ui, Tone::Bad, &i18n.t("bulk.report_failed"), true);
            ui.label(i18n.t(error.i18n_key()));
        }
        None => {}
    }
    report_undo_status(ui, flow, i18n);
}

/// The counts of a write that went through: matched, modified and inserted for an update, deleted
/// for a delete. `counts` is `[matched, modified, upserted, deleted]`.
fn count_labels(ui: &mut egui::Ui, kind: BulkKind, i18n: &I18n, counts: [u64; 4]) {
    let [matched, modified, upserted, deleted] = counts;
    let lines = match kind {
        BulkKind::Update => vec![
            ("bulk.report.matched", matched),
            ("bulk.report.modified", modified),
            ("bulk.report.upserted", upserted),
        ],
        BulkKind::Delete => vec![("bulk.report.deleted", deleted)],
    };
    for (key, n) in lines {
        ui.label(i18n.t_with(key, &[("n", &n.to_string())]));
    }
}
