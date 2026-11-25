//! The "Export documents" entry of a collection's context menu. It exports the whole collection,
//! so only the connection and a running export can close it: a collation on the shown page is no
//! reason. A closed entry shows its reason as a line under it, not on hover alone.
use super::sidebar_copy::TreeCtx;
use crate::I18n;
use crate::app::state::ConnStatus;
use crate::viewmodel::SidebarAction;
use crate::viewmodel::doc_export::doc_export_block;
use curlybrace_core::store::{Namespace, Query};
use eframe::egui;

/// The entry for `ns`; the action when it was clicked. Without a tree context nothing is known
/// about the connection, so the entry is closed as not connected.
pub fn export_entry(
    ui: &mut egui::Ui,
    ctx: Option<&TreeCtx>,
    ns: Namespace,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let block = match ctx {
        Some(ctx) => doc_export_block(
            ctx.state.status == ConnStatus::Connected,
            &Query::default(),
            ctx.state.doc_export.is_busy(),
        ),
        None => doc_export_block(false, &Query::default(), false),
    };
    let button = ui.add_enabled(
        block.is_none(),
        egui::Button::new(i18n.t("export.entry.menu")),
    );
    match block {
        Some(reason) => {
            let reason = i18n.t(reason.i18n_key());
            ui.label(egui::RichText::new(&reason).weak());
            button.on_disabled_hover_text(reason);
            None
        }
        None if button.clicked() => {
            ui.close();
            Some(SidebarAction::ExportDocuments(ns))
        }
        None => None,
    }
}
