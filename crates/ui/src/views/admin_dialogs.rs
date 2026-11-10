//! The modal of the sidebar's administration writes: drop (typed name), rename, duplicate and
//! create view. It draws the state it is given and reports what the person did; the reducer
//! decides what happens. While a write is in flight, or waits for the production warning, every
//! control is off. On a read-only connection the send button is off and says why.
use super::connection_badge::show_badge;
use super::error_label::show_error;
use super::index_dialogs::focus_once;
use super::tone_notice::tone_notice;
use super::typed_confirmation::{TypedConfirmation, show_typed_confirmation};
use crate::I18n;
use crate::theme::{
    FieldOptions, Tone, filled_button, gate, modal_frame, outlined_field, outlined_field_opts,
    text_button, typography,
};
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::{AdminAction, AdminDialog, AdminKind, AdminProblem, ConnectionBadge};
use eframe::egui;

const MODAL_WIDTH: f32 = 480.0;
const PIPELINE_ROWS: usize = 6;

/// What the modal needs besides its own state.
pub struct AdminContext<'a> {
    /// The label of the connection the dialog acts on.
    pub connection: &'a str,
    pub badge: &'a ConnectionBadge,
    /// A write is in flight or waits for the production warning.
    pub busy: bool,
}

fn kind_key(kind: AdminKind) -> &'static str {
    match kind {
        AdminKind::DropCollection => "drop_collection",
        AdminKind::DropDatabase => "drop_database",
        AdminKind::Rename => "rename",
        AdminKind::Duplicate => "duplicate",
        AdminKind::CreateView => "create_view",
    }
}

/// The i18n key of the dialog's title, message or confirm button for `kind`.
pub fn text_key(part: &str, kind: AdminKind) -> String {
    format!("admin.{part}.{}", kind_key(kind))
}

/// Draws the dialog; returns what the person did this frame.
pub fn show_admin_dialog(
    ctx: &egui::Context,
    dialog: &AdminDialog,
    context: &AdminContext<'_>,
    i18n: &I18n,
) -> Option<AdminAction> {
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("admin_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(MODAL_WIDTH);
            ui.horizontal(|ui| {
                show_badge(ui, context.badge, i18n);
                let title = i18n.t(&text_key("title", dialog.kind));
                ui.label(typography::dialog_title(ui, title));
            });
            ui.label(message(dialog, context, i18n));
            if context.badge.production {
                let line = i18n.t("write_warning.production_line");
                tone_notice(ui, Tone::Bad, &line, true);
            }
            ui.add_enabled_ui(!context.busy, |ui| {
                action = fields(ui, dialog, i18n);
            });
            problem_line(ui, dialog, i18n);
            if let Some(error) = &dialog.error {
                show_error(ui, error, i18n);
            }
            if context.badge.read_only {
                ui.label(i18n.t(READ_ONLY_REASON_KEY));
            }
            action = buttons(ui, dialog, context, i18n).or(action.take());
        });
    let dismissed = modal.should_close() && !context.busy;
    action.or(dismissed.then_some(AdminAction::Cancelled))
}

fn message(dialog: &AdminDialog, context: &AdminContext<'_>, i18n: &I18n) -> String {
    let collection = dialog.collection.as_deref().unwrap_or_default();
    i18n.t_with(
        &text_key("message", dialog.kind),
        &[
            ("db", &dialog.db),
            ("collection", collection),
            ("connection", context.connection),
        ],
    )
}

/// The inputs of the kind; returns the edit made this frame.
fn fields(ui: &mut egui::Ui, dialog: &AdminDialog, i18n: &I18n) -> Option<AdminAction> {
    match dialog.kind {
        AdminKind::DropCollection | AdminKind::DropDatabase => typed(ui, dialog, i18n),
        AdminKind::Rename => {
            let edit = name_field(ui, dialog, "admin.field.new_name", i18n);
            let mut replace = dialog.drop_target;
            let toggled = ui
                .checkbox(&mut replace, i18n.t("admin.field.replace_existing"))
                .changed();
            edit.or(toggled.then_some(AdminAction::DropTargetToggled(replace)))
        }
        AdminKind::Duplicate => name_field(ui, dialog, "admin.field.copy_name", i18n),
        AdminKind::CreateView => view_fields(ui, dialog, i18n),
    }
}

fn typed(ui: &mut egui::Ui, dialog: &AdminDialog, i18n: &I18n) -> Option<AdminAction> {
    let required = dialog.required_text().unwrap_or_default();
    let caption = i18n.t_with("admin.typed.accessible", &[("name", required)]);
    let prompt = TypedConfirmation {
        instruction: &i18n.t("admin.typed.instruction"),
        required,
        field_label: &i18n.t("admin.typed.field"),
        accessible_name: &caption,
    };
    let mut text = dialog.typed.clone();
    let input = show_typed_confirmation(ui, &prompt, &mut text);
    // The box takes the focus when the modal opens; a stray Enter cannot drop anything because
    // the button stays off until the exact name is typed.
    focus_once(ui, &input, ("admin_typed_focus", required));
    input.changed().then_some(AdminAction::Typed(text))
}

fn name_field(
    ui: &mut egui::Ui,
    dialog: &AdminDialog,
    label_key: &str,
    i18n: &I18n,
) -> Option<AdminAction> {
    let mut name = dialog.name.clone();
    let input = outlined_field(ui, &i18n.t(label_key), &mut name, "");
    focus_once(ui, &input, ("admin_name_focus", kind_key(dialog.kind)));
    input.changed().then_some(AdminAction::NameEdited(name))
}

fn view_fields(ui: &mut egui::Ui, dialog: &AdminDialog, i18n: &I18n) -> Option<AdminAction> {
    let edit = name_field(ui, dialog, "admin.field.view_name", i18n);
    let mut source = dialog.source.clone();
    let source_edit = outlined_field(ui, &i18n.t("admin.field.view_source"), &mut source, "")
        .changed()
        .then_some(AdminAction::SourceEdited(source));
    let mut pipeline = dialog.pipeline.clone();
    let options = FieldOptions {
        multiline_rows: Some(PIPELINE_ROWS),
        ..FieldOptions::default()
    };
    let label = i18n.t("admin.field.view_pipeline");
    let pipeline_edit = outlined_field_opts(ui, &label, &mut pipeline, "[]", options)
        .changed()
        .then_some(AdminAction::PipelineEdited(pipeline));
    edit.or(source_edit).or(pipeline_edit)
}

/// Says what is wrong with what was typed; the missing-value and unconfirmed-drop cases are
/// clear from the empty field and the disabled button, and stay quiet.
fn problem_line(ui: &mut egui::Ui, dialog: &AdminDialog, i18n: &I18n) {
    match dialog.problem() {
        Some(AdminProblem::Required | AdminProblem::ConfirmMismatch) | None => {}
        Some(problem) => {
            ui.colored_label(ui.visuals().error_fg_color, i18n.t(problem.key()));
        }
    }
}

fn buttons(
    ui: &mut egui::Ui,
    dialog: &AdminDialog,
    context: &AdminContext<'_>,
    i18n: &I18n,
) -> Option<AdminAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let cancel = gate(ui, !context.busy, |ui| {
            text_button(ui, i18n.t("common.cancel"))
        });
        let enabled = dialog.ready() && !context.busy && !context.badge.read_only;
        let send = gate(ui, enabled, |ui| {
            filled_button(ui, i18n.t(&text_key("confirm", dialog.kind)))
        });
        if cancel.clicked() {
            action = Some(AdminAction::Cancelled);
        }
        if send.clicked() {
            action = Some(AdminAction::Confirmed);
        }
    });
    action
}
