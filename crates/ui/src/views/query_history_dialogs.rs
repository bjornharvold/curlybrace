//! The dialogs of the History popup: Save as favourite and Rename (a name), Delete and Clear
//! history (a question). One modal at a time, drawn from the popup's `HistoryUi`. A name dialog
//! puts the focus in its name field, Enter there confirms (when the name is usable) and Escape
//! or a click outside cancels. A question starts on Cancel, so a stray Enter changes nothing.
use super::index_dialogs::focus_once;
use super::pane_scope::{note_modal, scoped_id};
use crate::I18n;
use crate::theme::{
    filled_button, gate, modal_frame, outlined_field_with, text_button, typography,
};
use crate::viewmodel::query_history::{Dialog, HistoryAction, HistoryUi, error_key, name_problem};
use curlybrace_core::query_history::{QueryHistory, SavedError};
use eframe::egui;
use uuid::Uuid;

const MODAL_WIDTH: f32 = 420.0;

/// The collection the dialogs act on: the connection, database and collection names.
pub type Target<'a> = (Uuid, &'a str, &'a str);

/// A dialog that asks for a name.
struct NameDialog<'a> {
    salt: &'static str,
    title: &'static str,
    confirm: &'static str,
    name: &'a str,
    problem: Option<SavedError>,
    failed: Option<&'static str>,
}

/// A question answered with Cancel or one confirming button.
struct Question {
    salt: &'static str,
    title: &'static str,
    message: String,
    confirm: &'static str,
    failed: Option<&'static str>,
}

/// Draws the open dialog, if any; returns what the person did. A dialog that names a favourite
/// which is gone is cancelled.
pub fn show_history_dialog(
    ctx: &egui::Context,
    state: &HistoryUi,
    (store, target): (&QueryHistory, Target<'_>),
    i18n: &I18n,
) -> Option<HistoryAction> {
    let dialog = state.dialog.as_ref()?;
    let failed = state.error;
    let favourite = |id: &Uuid| {
        store
            .favourites(target.0)
            .iter()
            .find(|f| f.id == *id)
            .map(|f| f.name.clone())
    };
    match dialog {
        Dialog::Star { name, .. } => name_dialog(
            ctx,
            &NameDialog {
                salt: "star",
                title: "history.dialog.star_title",
                confirm: "history.dialog.star_confirm",
                name,
                problem: name_problem(store, target, name, None),
                failed,
            },
            i18n,
        ),
        Dialog::Rename { id, name } => match favourite(id) {
            Some(_) => name_dialog(
                ctx,
                &NameDialog {
                    salt: "rename",
                    title: "history.dialog.rename_title",
                    confirm: "history.dialog.rename_confirm",
                    name,
                    problem: name_problem(store, target, name, Some(*id)),
                    failed,
                },
                i18n,
            ),
            None => Some(HistoryAction::DialogCancelled),
        },
        Dialog::Delete { id } => match favourite(id) {
            Some(name) => question(
                ctx,
                Question {
                    salt: "delete",
                    title: "history.dialog.delete_title",
                    message: i18n.t_with("history.dialog.delete_message", &[("name", &name)]),
                    confirm: "history.dialog.delete_confirm",
                    failed,
                },
                i18n,
            ),
            None => Some(HistoryAction::DialogCancelled),
        },
        Dialog::Clear => question(
            ctx,
            Question {
                salt: "clear",
                title: "history.dialog.clear_title",
                message: i18n.t("history.dialog.clear_message"),
                confirm: "history.dialog.clear_confirm",
                failed,
            },
            i18n,
        ),
    }
}

/// `problem` as the message under a name field. An empty name is no message: the confirm button
/// is simply off until something is typed.
fn problem_message(problem: Option<SavedError>, i18n: &I18n) -> Option<String> {
    problem
        .filter(|p| *p != SavedError::EmptyName)
        .map(|p| i18n.t(error_key(p)))
}

fn show_failure(ui: &mut egui::Ui, failed: Option<&'static str>, i18n: &I18n) {
    if let Some(key) = failed {
        ui.colored_label(ui.visuals().error_fg_color, i18n.t(key));
    }
}

fn name_dialog(ctx: &egui::Context, d: &NameDialog<'_>, i18n: &I18n) -> Option<HistoryAction> {
    let mut action = None;
    let usable = d.problem.is_none();
    let modal = egui::Modal::new(note_modal(ctx, scoped_id(ctx, ("history_dialog", d.salt))))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(MODAL_WIDTH);
            ui.label(typography::dialog_title(ui, i18n.t(d.title)));
            let mut text = d.name.to_string();
            let message = problem_message(d.problem, i18n);
            let label = i18n.t("history.dialog.name_label");
            let field = outlined_field_with(ui, &label, &mut text, "", message.as_deref());
            if field.changed() {
                action = Some(HistoryAction::NameEdited(text));
            }
            focus_once(ui, &field, ("history_focus", d.salt));
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            show_failure(ui, d.failed, i18n);
            ui.horizontal(|ui| {
                if text_button(ui, i18n.t("common.cancel")).clicked() {
                    action = Some(HistoryAction::DialogCancelled);
                }
                let confirm = gate(ui, usable, |ui| filled_button(ui, i18n.t(d.confirm)));
                if usable && (confirm.clicked() || entered) {
                    action = Some(HistoryAction::Confirmed);
                }
            });
        });
    action.or(modal
        .should_close()
        .then_some(HistoryAction::DialogCancelled))
}

fn question(ctx: &egui::Context, q: Question, i18n: &I18n) -> Option<HistoryAction> {
    let mut action = None;
    let modal = egui::Modal::new(note_modal(ctx, scoped_id(ctx, ("history_dialog", q.salt))))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(MODAL_WIDTH);
            ui.label(typography::dialog_title(ui, i18n.t(q.title)));
            ui.label(&q.message);
            show_failure(ui, q.failed, i18n);
            ui.horizontal(|ui| {
                let cancel = text_button(ui, i18n.t("common.cancel"));
                focus_once(ui, &cancel, ("history_focus", q.salt));
                if cancel.clicked() {
                    action = Some(HistoryAction::DialogCancelled);
                }
                if filled_button(ui, i18n.t(q.confirm)).clicked() {
                    action = Some(HistoryAction::Confirmed);
                }
            });
        });
    action.or(modal
        .should_close()
        .then_some(HistoryAction::DialogCancelled))
}
