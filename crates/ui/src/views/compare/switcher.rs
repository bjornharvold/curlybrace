//! The Browse | Compare switch next to the connection button.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::app::state::Workspace;
use crate::theme::segmented;
use eframe::egui;

/// `Show` / `Hide` for the entry that is not the current workspace. With the connection manager
/// up (`Workspace::Connections`) neither entry is selected, so either one leaves it. The hints
/// (what each side shows) are the tooltip of the whole switch: one widget draws both segments.
pub fn show_workspace_switcher(
    ui: &mut egui::Ui,
    current: Workspace,
    i18n: &I18n,
) -> Option<CompareAction> {
    let items = [
        (Workspace::Browse, i18n.t("workspace.browse")),
        (Workspace::Compare, i18n.t("workspace.compare")),
    ];
    let hints = format!(
        "{}: {}\n{}: {}",
        items[0].1,
        i18n.t("workspace.browse_hint"),
        items[1].1,
        i18n.t("workspace.compare_hint"),
    );
    let shown = ui.horizontal(|ui| segmented(ui, &items, current));
    shown.response.on_hover_text(hints);
    shown.inner.map(|picked| match picked {
        Workspace::Compare => CompareAction::Show,
        _ => CompareAction::Hide,
    })
}
