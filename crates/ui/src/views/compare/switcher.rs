//! The Browse | Compare switch next to the connection button.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::app::state::Workspace;
use eframe::egui;

/// `Show` / `Hide` for the entry that is not the current workspace.
pub fn show_workspace_switcher(
    ui: &mut egui::Ui,
    current: Workspace,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let browse = ui.selectable_label(current == Workspace::Browse, i18n.t("workspace.browse"));
        if browse
            .on_hover_text(i18n.t("workspace.browse_hint"))
            .clicked()
        {
            action = Some(CompareAction::Hide);
        }
        let compare =
            ui.selectable_label(current == Workspace::Compare, i18n.t("workspace.compare"));
        if compare
            .on_hover_text(i18n.t("workspace.compare_hint"))
            .clicked()
        {
            action = Some(CompareAction::Show);
        }
    });
    action
}
