//! The Browse | Compare | Shell switch next to the connection button.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::app::reducer::Input;
use crate::app::shell_actions::ShellAction;
use crate::app::state::Workspace;
use crate::theme::segmented_where;
use eframe::egui;

/// The input for the entry picked when it is not the current workspace: Compare shows the
/// compare workspace, Shell opens the active session's Shell tab, Browse goes back to the
/// documents. With the connection manager up (`Workspace::Connections`) no entry is selected,
/// so any one leaves it. Shell is disabled while the active session is not `connected` (it would
/// open nothing). The hints (what each entry shows, and why Shell is off) are the tooltip of the
/// whole switch: one widget draws all segments.
pub fn show_workspace_switcher(
    ui: &mut egui::Ui,
    current: Workspace,
    connected: bool,
    i18n: &I18n,
) -> Option<Input> {
    let items = [
        (Workspace::Browse, i18n.t("workspace.browse")),
        (Workspace::Compare, i18n.t("workspace.compare")),
        (Workspace::Shell, i18n.t("workspace.shell")),
    ];
    let enabled = |entry: &Workspace| connected || *entry != Workspace::Shell;
    let shown = ui.horizontal(|ui| segmented_where(ui, &items, current, enabled));
    shown.response.on_hover_text(hints(i18n, connected));
    shown.inner.map(|picked| match picked {
        Workspace::Compare => Input::Compare(CompareAction::Show),
        Workspace::Shell => Input::Shell(ShellAction::Open),
        _ => Input::Compare(CompareAction::Hide),
    })
}

/// One translated `name: hint` line per entry.
fn hints(i18n: &I18n, connected: bool) -> String {
    let shell_hint = match connected {
        true => "workspace.shell_hint",
        false => "workspace.shell_hint_not_connected",
    };
    [
        ("workspace.browse", "workspace.browse_hint"),
        ("workspace.compare", "workspace.compare_hint"),
        ("workspace.shell", shell_hint),
    ]
    .iter()
    .map(|(name, hint)| {
        let (name, hint) = (i18n.t(name), i18n.t(hint));
        i18n.t_with(
            "workspace.switcher_hint_line",
            &[("name", &name), ("hint", &hint)],
        )
    })
    .collect::<Vec<_>>()
    .join("\n")
}

#[cfg(test)]
#[path = "switcher_tests.rs"]
mod tests;
