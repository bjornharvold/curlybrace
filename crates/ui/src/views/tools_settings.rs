//! The global tool paths dialog: one editor per tool and a Close button. Opened from the gear.
use super::tool_row::{RowContext, show_tool_row};
use crate::I18n;
use crate::app::tools_state::{ProbeScope, ToolsAction, ToolsState};
use crate::viewmodel::tool_rows::tool_row;
use curlybrace_core::tools::ToolKind;
use eframe::egui;

const DIALOG_WIDTH: f32 = 560.0;

/// Draws the dialog while it is open and returns what was done, in order. Close and Escape close it; a click outside does not, so a
/// stray click never loses a half-typed path.
pub fn show_tools_settings(
    ctx: &egui::Context,
    tools: &ToolsState,
    i18n: &I18n,
) -> Vec<ToolsAction> {
    let Some(dialog) = tools.dialog() else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    let mut closed = false;
    let modal = egui::Modal::new(egui::Id::new("tool_settings")).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);
        ui.heading(i18n.t("tools.heading"));
        ui.weak(i18n.t("tools.intro"));
        for kind in ToolKind::ALL {
            ui.separator();
            let row = tool_row(tools, ProbeScope::Global, kind, None);
            let context = RowContext {
                scope: ProbeScope::Global,
                server: None,
                suggestions: dialog.suggestions(kind),
            };
            actions.extend(show_tool_row(ui, &row, &context, i18n));
        }
        ui.separator();
        closed = ui.button(i18n.t("common.close")).clicked();
    });
    let escape = modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    // A path typed just before Close is applied in the frame its box loses focus, so it is
    // listed first: closing never drops it.
    if closed || escape {
        actions.push(ToolsAction::CloseSettings);
    }
    actions
}
