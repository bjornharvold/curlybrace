//! The Browse | Compare | Shell switch next to the connection button.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::app::reducer::Input;
use crate::app::shell_actions::ShellAction;
use crate::app::state::Workspace;
use crate::theme::segmented;
use eframe::egui;

/// The input for the entry picked when it is not the current workspace: Compare shows the
/// compare workspace, Shell opens the active session's Shell tab, Browse goes back to the
/// documents. With the connection manager up (`Workspace::Connections`) no entry is selected,
/// so any one leaves it. The hints (what each entry shows) are the tooltip of the whole switch:
/// one widget draws all segments.
pub fn show_workspace_switcher(
    ui: &mut egui::Ui,
    current: Workspace,
    i18n: &I18n,
) -> Option<Input> {
    let items = [
        (Workspace::Browse, i18n.t("workspace.browse")),
        (Workspace::Compare, i18n.t("workspace.compare")),
        (Workspace::Shell, i18n.t("workspace.shell")),
    ];
    let hints = format!(
        "{}: {}\n{}: {}\n{}: {}",
        items[0].1,
        i18n.t("workspace.browse_hint"),
        items[1].1,
        i18n.t("workspace.compare_hint"),
        items[2].1,
        i18n.t("workspace.shell_hint"),
    );
    let shown = ui.horizontal(|ui| segmented(ui, &items, current));
    shown.response.on_hover_text(hints);
    shown.inner.map(|picked| match picked {
        Workspace::Compare => Input::Compare(CompareAction::Show),
        Workspace::Shell => Input::Shell(ShellAction::Open),
        _ => Input::Compare(CompareAction::Hide),
    })
}
