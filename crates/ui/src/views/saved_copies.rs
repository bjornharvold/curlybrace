//! The list of saved undo copies: where they are kept, what each one is, and a Delete with a
//! question before it. Draws the state it is given and returns what the person did.
use super::connections_list::time_text;
use super::tone_notice::tone_notice;
use crate::I18n;
use crate::app::saved_copies::{CopiesAction, SavedCopies};
use crate::theme::{Tone, filled_button, modal_frame, text_button, tonal_button, typography};
use crate::viewmodel::index_detail::size_parts;
use curlybrace_core::snapshot::{RETENTION_SECS, SnapshotInfo, SnapshotOp};
use eframe::egui;
use std::path::Path;

const DIALOG_WIDTH: f32 = 640.0;
const LIST_MAX_HEIGHT: f32 = 320.0;
const SECS_PER_DAY: i64 = 24 * 60 * 60;

pub fn show_saved_copies(
    ctx: &egui::Context,
    copies: &SavedCopies,
    folder: Option<&Path>,
    i18n: &I18n,
) -> Option<CopiesAction> {
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("saved_copies"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.label(typography::dialog_title(ui, i18n.t("copies.title")));
            intro(ui, folder, i18n);
            action = list(ui, copies, i18n);
            ui.separator();
            if filled_button(ui, i18n.t("common.close")).clicked() {
                action = Some(CopiesAction::Close);
            }
        });
    let escape = modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    action.or(escape.then_some(CopiesAction::Close))
}

fn intro(ui: &mut egui::Ui, folder: Option<&Path>, i18n: &I18n) {
    let place = match folder {
        Some(dir) => dir.display().to_string(),
        None => i18n.t("bulk.undo.folder_unknown"),
    };
    let days = (RETENTION_SECS / SECS_PER_DAY).to_string();
    let text = i18n.t_with("copies.intro", &[("folder", &place), ("days", &days)]);
    tone_notice(ui, Tone::Neutral, &text, false);
}

fn list(ui: &mut egui::Ui, copies: &SavedCopies, i18n: &I18n) -> Option<CopiesAction> {
    let Some(rows) = &copies.list else {
        ui.label(i18n.t("copies.loading"));
        return None;
    };
    if rows.is_empty() {
        ui.label(i18n.t("copies.empty"));
        return None;
    }
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("saved_copies_list")
        .max_height(LIST_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for info in rows {
                action = row(ui, info, copies, i18n).or(action.take());
            }
        });
    action
}

/// The text that says what the copy is, shared by the row and the name of its button.
fn describe_copy(info: &SnapshotInfo, i18n: &I18n) -> (String, String) {
    let ns = format!("{}.{}", info.ns.db, info.ns.collection);
    let op = match info.op {
        SnapshotOp::Update => i18n.t("copies.op_update"),
        SnapshotOp::Delete => i18n.t("copies.op_delete"),
    };
    let when = time_text(Some(info.created_unix), i18n);
    let (key, n) = size_parts(Some(info.bytes));
    let size = i18n.t_with(key, &[("n", &n)]);
    let line = i18n.t_with(
        "copies.row",
        &[("ns", &ns), ("op", &op), ("when", &when), ("size", &size)],
    );
    let name = i18n.t_with("copies.delete_named", &[("ns", &ns), ("when", &when)]);
    (line, name)
}

fn row(
    ui: &mut egui::Ui,
    info: &SnapshotInfo,
    copies: &SavedCopies,
    i18n: &I18n,
) -> Option<CopiesAction> {
    let (line, name) = describe_copy(info, i18n);
    let mut action = None;
    ui.add_space(4.0);
    ui.label(line);
    ui.horizontal_wrapped(|ui| {
        if copies.asking == Some(info.run) {
            action = question(ui, info, i18n);
            return;
        }
        let delete = ui.add_enabled_ui(!copies.busy(), |ui| {
            text_button(ui, i18n.t("copies.delete"))
        });
        let delete = delete.inner;
        delete.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, !copies.busy(), name.clone())
        });
        if delete.clicked() {
            action = Some(CopiesAction::Ask(info.run));
        }
    });
    action
}

/// The question before a deletion: the copy cannot be brought back once it is deleted, and
/// neither can the undo it makes possible.
fn question(ui: &mut egui::Ui, info: &SnapshotInfo, i18n: &I18n) -> Option<CopiesAction> {
    let mut action = None;
    ui.label(egui::RichText::new(i18n.t("copies.confirm")).strong());
    if tonal_button(ui, i18n.t("copies.confirm_delete")).clicked() {
        action = Some(CopiesAction::Delete(info.run));
    }
    if text_button(ui, i18n.t("copies.keep")).clicked() {
        action = Some(CopiesAction::Keep);
    }
    action
}
