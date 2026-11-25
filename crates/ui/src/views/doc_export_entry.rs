//! The export entries of a collection's context menu. "Export documents" exports the whole
//! collection, so only the connection and a running export can close it: a collation on the shown
//! page is no reason. "Export documents matching the filter" is offered only on the collection
//! shown in the documents pane while a filter is applied, and a collation closes it. A closed
//! entry shows its reason as a line under it, not on hover alone.
use super::sidebar_copy::TreeCtx;
use crate::I18n;
use crate::app::state::ConnStatus;
use crate::viewmodel::SidebarAction;
use crate::viewmodel::doc_export::{DocExportBlock, doc_export_block};
use curlybrace_core::store::{Namespace, Query};
use eframe::egui;

/// The entries for `ns`; the action of the one clicked. Without a tree context nothing is known
/// about the connection, so the whole-collection entry is closed as not connected and the
/// filter entry is absent.
pub fn export_entries(
    ui: &mut egui::Ui,
    ctx: Option<&TreeCtx>,
    ns: &Namespace,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let whole = match ctx {
        Some(ctx) => {
            let state = ctx.state;
            let block = doc_export_block(
                state.status == ConnStatus::Connected,
                &Query::default(),
                state.doc_export.is_busy(),
            );
            entry(ui, "export.entry.menu", block, i18n)
                .then(|| SidebarAction::ExportDocuments(ns.clone()))
        }
        None => {
            let block = doc_export_block(false, &Query::default(), false);
            entry(ui, "export.entry.menu", block, i18n);
            None
        }
    };
    let matching = ctx
        .filter(|c| c.state.selected_ns() == Some(ns) && !c.state.page_query.filter.is_empty())
        .and_then(|c| {
            let state = c.state;
            let block = doc_export_block(
                state.status == ConnStatus::Connected,
                &state.page_query,
                state.doc_export.is_busy(),
            );
            entry(ui, "export.entry.query", block, i18n)
                .then(|| SidebarAction::ExportMatching(ns.clone()))
        });
    matching.or(whole)
}

/// One entry: disabled with its reason as a line and hover text when `block` holds. True when
/// clicked (the menu is closed then).
fn entry(ui: &mut egui::Ui, key: &str, block: Option<DocExportBlock>, i18n: &I18n) -> bool {
    let button = ui.add_enabled(block.is_none(), egui::Button::new(i18n.t(key)));
    match block {
        Some(reason) => {
            let reason = i18n.t(reason.i18n_key());
            ui.label(egui::RichText::new(&reason).weak());
            button.on_disabled_hover_text(reason);
            false
        }
        None if button.clicked() => {
            ui.close();
            true
        }
        None => false,
    }
}
