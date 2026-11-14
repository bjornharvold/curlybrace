//! The dialogs of the saved pipelines: Save (a name, then a replace prompt when the name is
//! taken), the open prompt that guards unsaved changes, Rename and Delete. One modal at a time,
//! drawn from the draft's `SavedUi`. A name dialog puts the focus in its name field, Enter there
//! confirms (when the name is usable) and Escape or a click outside cancels. A prompt that
//! replaces or removes something starts on Cancel, so a stray Enter changes nothing.
use super::index_dialogs::focus_once;
use crate::I18n;
use crate::app::AggregateAction;
use crate::theme::{
    filled_button, gate, modal_frame, outlined_field_with, text_button, typography,
};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_saved::{SavedDialog, name_problem, saved_error_key};
use curlybrace_core::saved_pipeline::{SavedError, SavedPipeline, same_name};
use eframe::egui;
use uuid::Uuid;

const MODAL_WIDTH: f32 = 420.0;

/// A dialog that asks for a name.
struct NameDialog<'a> {
    salt: &'static str,
    title: &'static str,
    confirm: &'static str,
    name: &'a str,
    problem: Option<SavedError>,
    /// The message of the last failed confirm, shown under the field.
    failed: Option<&'static str>,
    edited: fn(String) -> AggregateAction,
    confirmed: AggregateAction,
}

/// A question answered with Cancel or one confirming button.
struct Prompt {
    salt: &'static str,
    title: &'static str,
    message: String,
    confirm: &'static str,
    confirmed: AggregateAction,
    failed: Option<&'static str>,
}

/// Draws the open dialog of `draft`, if any; returns what the person did. A dialog that names a
/// pipeline which is gone is cancelled.
pub fn show_saved_dialog(
    ctx: &egui::Context,
    draft: &AggregateState,
    entries: &[SavedPipeline],
    i18n: &I18n,
) -> Option<AggregateAction> {
    let dialog = draft.saved.dialog.as_ref()?;
    let failed = draft.saved.error;
    let gone = || Some(AggregateAction::DialogCancelled);
    let named = |id: &Uuid| entries.iter().find(|p| p.id == *id).map(|p| p.name.clone());
    match dialog {
        SavedDialog::Save {
            name,
            replace_prompt: false,
        } => name_dialog(
            ctx,
            &NameDialog {
                salt: "save",
                title: "aggregate.saved.save_title",
                confirm: "aggregate.saved.save",
                name,
                problem: name_problem(entries, name, None, false),
                failed,
                edited: AggregateAction::SaveNameEdited,
                confirmed: AggregateAction::SaveConfirmed,
            },
            i18n,
        ),
        SavedDialog::Save {
            name,
            replace_prompt: true,
        } => {
            // The pipeline that is replaced keeps its stored spelling; name it as stored.
            let stored = entries
                .iter()
                .find(|p| same_name(&p.name, name))
                .map_or_else(|| name.trim().to_string(), |p| p.name.clone());
            prompt_dialog(
                ctx,
                Prompt {
                    salt: "replace",
                    title: "aggregate.saved.save_title",
                    message: i18n.t_with("aggregate.saved.replace_message", &[("name", &stored)]),
                    confirm: "aggregate.saved.replace_confirm",
                    confirmed: AggregateAction::SaveConfirmed,
                    failed,
                },
                i18n,
            )
        }
        SavedDialog::ConfirmOpen { id } => named(id).map_or_else(gone, |name| {
            prompt_dialog(
                ctx,
                Prompt {
                    salt: "open",
                    title: "aggregate.saved.open_title",
                    message: i18n.t_with("aggregate.saved.open_message", &[("name", &name)]),
                    confirm: "aggregate.saved.open_confirm",
                    confirmed: AggregateAction::OpenConfirmed,
                    failed,
                },
                i18n,
            )
        }),
        SavedDialog::Rename { id, name } => named(id).map_or_else(gone, |_| {
            name_dialog(
                ctx,
                &NameDialog {
                    salt: "rename",
                    title: "aggregate.saved.rename_title",
                    confirm: "aggregate.saved.rename_confirm",
                    name,
                    problem: name_problem(entries, name, Some(*id), true),
                    failed,
                    edited: AggregateAction::RenameEdited,
                    confirmed: AggregateAction::RenameConfirmed,
                },
                i18n,
            )
        }),
        SavedDialog::ConfirmDelete { id } => named(id).map_or_else(gone, |name| {
            prompt_dialog(
                ctx,
                Prompt {
                    salt: "delete",
                    title: "aggregate.saved.delete_title",
                    message: i18n.t_with("aggregate.saved.delete_message", &[("name", &name)]),
                    confirm: "aggregate.saved.delete_confirm",
                    confirmed: AggregateAction::DeleteConfirmed,
                    failed,
                },
                i18n,
            )
        }),
    }
}

/// `problem` as the message under a name field. An empty name is no message: the confirm button
/// is simply off until something is typed.
fn problem_message(problem: Option<SavedError>, i18n: &I18n) -> Option<String> {
    problem
        .filter(|p| *p != SavedError::EmptyName)
        .map(|p| i18n.t(saved_error_key(p)))
}

fn show_failure(ui: &mut egui::Ui, failed: Option<&'static str>, i18n: &I18n) {
    if let Some(key) = failed {
        ui.colored_label(ui.visuals().error_fg_color, i18n.t(key));
    }
}

fn name_dialog(ctx: &egui::Context, d: &NameDialog<'_>, i18n: &I18n) -> Option<AggregateAction> {
    let mut action = None;
    let usable = d.problem.is_none();
    let modal = egui::Modal::new(egui::Id::new(("aggregate_saved_dialog", d.salt)))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(MODAL_WIDTH);
            ui.label(typography::dialog_title(ui, i18n.t(d.title)));
            let mut text = d.name.to_string();
            let message = problem_message(d.problem, i18n);
            let label = i18n.t("aggregate.saved.name_label");
            let field = outlined_field_with(ui, &label, &mut text, "", message.as_deref());
            if field.changed() {
                action = Some((d.edited)(text));
            }
            focus_once(ui, &field, ("aggregate_saved_focus", d.salt));
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            show_failure(ui, d.failed, i18n);
            ui.horizontal(|ui| {
                if text_button(ui, i18n.t("common.cancel")).clicked() {
                    action = Some(AggregateAction::DialogCancelled);
                }
                let confirm = gate(ui, usable, |ui| filled_button(ui, i18n.t(d.confirm)));
                if usable && (confirm.clicked() || entered) {
                    action = Some(d.confirmed.clone());
                }
            });
        });
    action.or(modal
        .should_close()
        .then_some(AggregateAction::DialogCancelled))
}

fn prompt_dialog(ctx: &egui::Context, p: Prompt, i18n: &I18n) -> Option<AggregateAction> {
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new(("aggregate_saved_dialog", p.salt)))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(MODAL_WIDTH);
            ui.label(typography::dialog_title(ui, i18n.t(p.title)));
            ui.label(&p.message);
            show_failure(ui, p.failed, i18n);
            ui.horizontal(|ui| {
                let cancel = text_button(ui, i18n.t("common.cancel"));
                focus_once(ui, &cancel, ("aggregate_saved_focus", p.salt));
                if cancel.clicked() {
                    action = Some(AggregateAction::DialogCancelled);
                }
                if filled_button(ui, i18n.t(p.confirm)).clicked() {
                    action = Some(p.confirmed.clone());
                }
            });
        });
    action.or(modal
        .should_close()
        .then_some(AggregateAction::DialogCancelled))
}
