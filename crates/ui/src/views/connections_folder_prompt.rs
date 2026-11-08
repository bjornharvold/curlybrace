//! The folder name prompt: moves a connection into a folder or renames a folder.
use super::connections::{ConnectionAction, ConnectionsState, FolderPrompt};
use crate::I18n;
use crate::theme::{
    FieldOptions, filled_button, modal_frame, outlined_field_opts, text_button, tonal_button,
    typography,
};
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
    // Set by each opening, so it holds however the previous prompt was closed.
    let first_frame = std::mem::take(&mut state.folder_prompt_focus_pending);
    let prompt = state.folder_prompt.as_mut()?;
    let (title_key, is_move, input) = match prompt {
        FolderPrompt::Move { input, .. } => ("connections.folder_move_title", true, input),
        FolderPrompt::Rename { input, .. } => ("connections.folder_rename_title", false, input),
    };
    let (mut confirmed, mut cancelled) = (false, false);
    let title = i18n.t(title_key);
    let frame = modal_frame(ctx);
    let modal = egui::Modal::new(egui::Id::new("connection_folder_prompt"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.label(typography::dialog_title(ui, title.clone()));
            // The prompt's title is also the field's label, so its accessible name is unchanged.
            let field = outlined_field_opts(ui, &title, input, "", FieldOptions::default());
            if first_frame {
                // Only when the prompt opens: never again, so Tab and clicks move focus freely.
                field.request_focus();
            }
            let blank = !is_move && input.trim().is_empty();
            confirmed =
                !blank && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(error) = &error {
                super::error_label::show_error(ui, error, i18n);
            }
            if is_move {
                ui.horizontal_wrapped(|ui| {
                    for name in &folders {
                        if tonal_button(ui, name).clicked() {
                            *input = name.clone();
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                cancelled = text_button(ui, i18n.t("common.cancel")).clicked();
                confirmed |= ui
                    .add_enabled_ui(!blank, |ui| filled_button(ui, i18n.t("common.ok")))
                    .inner
                    .clicked();
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
