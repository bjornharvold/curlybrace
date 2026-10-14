//! The folder name prompt: moves a connection into a folder or renames a folder.
use super::connections::{ConnectionAction, ConnectionsState, FolderPrompt};
use crate::I18n;
use eframe::egui;

/// Modal with one text field. Escape or a click outside cancels. The handler validates the
/// name and closes the prompt on success; a rejection is shown here, next to the input, and the
/// typed text is kept. A blank rename cannot be confirmed.
pub(super) fn show_folder_prompt(
    ctx: &egui::Context,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let folders = state.folder_names();
    let error = state.error.clone();
    let prompt = state.folder_prompt.as_mut()?;
    let (title_key, is_move, input) = match prompt {
        FolderPrompt::Move { input, .. } => ("connections.folder_move_title", true, input),
        FolderPrompt::Rename { input, .. } => ("connections.folder_rename_title", false, input),
    };
    let (mut confirmed, mut cancelled) = (false, false);
    let modal = egui::Modal::new(egui::Id::new("connection_folder_prompt")).show(ctx, |ui| {
        let title = ui.heading(i18n.t(title_key));
        let field = ui
            .add(egui::TextEdit::singleline(input))
            .labelled_by(title.id);
        let blank = !is_move && input.trim().is_empty();
        confirmed = !blank && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if let Some(error) = &error {
            super::error_label::show_error(ui, error, i18n);
        }
        if is_move {
            ui.horizontal_wrapped(|ui| {
                for name in &folders {
                    if ui.button(name).clicked() {
                        *input = name.clone();
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            cancelled = ui.button(i18n.t("common.cancel")).clicked();
            let ok = egui::Button::new(i18n.t("common.ok"));
            confirmed |= ui.add_enabled(!blank, ok).clicked();
        });
    });
    if confirmed {
        return state.folder_prompt_confirmed();
    }
    if cancelled || modal.should_close() {
        state.close_folder_prompt();
    }
    None
}
