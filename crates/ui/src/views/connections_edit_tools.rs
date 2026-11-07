//! The tool paths section of the connection dialog: for each tool, follow the global settings or
//! use this connection's own. Only a saved connection has a section, because the choices are
//! kept by connection id and an unsaved one would leave a stray entry behind. A tool older than
//! the server the person last tested against is called out.
use super::connections::{ConnectionAction, ConnectionsState, TestState};
use super::tool_row::{RowContext, show_tool_row};
use crate::I18n;
use crate::app::tools_state::{ProbeScope, ToolsState};
use crate::viewmodel::tool_rows::tool_row;
use curlybrace_core::tools::{ServerVersion, ToolKind};
use eframe::egui;

/// The server version of the last successful test of the form as it is now.
fn tested_server(state: &ConnectionsState) -> Option<ServerVersion> {
    match &state.test {
        TestState::Done {
            outcome: Ok(report),
            ..
        } => report
            .server_version
            .as_deref()
            .and_then(ServerVersion::parse),
        _ => None,
    }
}

pub(super) fn show_tools_section(
    ui: &mut egui::Ui,
    state: &ConnectionsState,
    tools: &ToolsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let id = state.form.id;
    let saved = state.connections.iter().any(|c| c.id == id);
    let server = tested_server(state);
    let mut action = None;
    egui::CollapsingHeader::new(i18n.t("tools.heading"))
        .id_salt("connection_tools")
        .default_open(false)
        .show(ui, |ui| {
            if !saved {
                ui.weak(i18n.t("tools.save_first"));
                return;
            }
            let scope = ProbeScope::Connection(id);
            for kind in ToolKind::ALL {
                let row = tool_row(tools, scope, kind, server);
                let context = RowContext {
                    scope,
                    server,
                    suggestions: &[],
                };
                action = show_tool_row(ui, &row, &context, i18n)
                    .map(ConnectionAction::Tools)
                    .or(action.take());
                ui.separator();
            }
        });
    action
}
