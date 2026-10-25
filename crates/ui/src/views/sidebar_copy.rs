//! Selection, Copy and Paste in the sidebar: how a click or a key changes the selection, and the
//! Copy and Paste entries of the row menus. A thin layer over `viewmodel::sidebar_selection` and
//! `viewmodel::copy_menu`, which decide everything; this only reads egui input and draws.
use crate::I18n;
use crate::app::state::AppState;
use crate::viewmodel::copy_menu::PASTE_KEY;
use crate::viewmodel::{
    NodeRef, PasteEntry, PasteTarget, SelectMode, SidebarAction, paste_target_for,
};
use curlybrace_core::store::Namespace;
use eframe::egui;
use uuid::Uuid;

/// What a root's tree needs to know beyond its own state: its session and the app-wide selection
/// and clipboard.
pub struct TreeCtx<'a> {
    pub state: &'a AppState,
    pub session: Uuid,
}

impl TreeCtx<'_> {
    fn is_selected(&self, node: &NodeRef) -> bool {
        self.state.selection.is_selected(node)
    }
}

/// The node of database `db` in the tree of `ctx` (a placeholder session without one, where
/// nothing is selectable).
pub fn database_node(ctx: Option<&TreeCtx>, db: &str) -> NodeRef {
    NodeRef::Database {
        session: ctx.map_or(Uuid::nil(), |c| c.session),
        db: db.to_string(),
    }
}

/// The node of a collection, like [`database_node`].
pub fn collection_node(ctx: Option<&TreeCtx>, db: &str, collection: &str) -> NodeRef {
    NodeRef::Collection {
        session: ctx.map_or(Uuid::nil(), |c| c.session),
        ns: Namespace::new(db, collection),
    }
}

/// Whether `node` is highlighted as part of the selection.
pub fn is_selected(ctx: Option<&TreeCtx>, node: &NodeRef) -> bool {
    ctx.is_some_and(|c| c.is_selected(node))
}

/// What a click with these modifiers does to the selection; `None` is a plain click.
pub fn click_mode(modifiers: &egui::Modifiers) -> Option<SelectMode> {
    if modifiers.command {
        Some(SelectMode::Toggle)
    } else if modifiers.shift {
        Some(SelectMode::Extend)
    } else {
        None
    }
}

/// What moving the keyboard focus onto a row does to the selection: the row alone, or the range
/// with Shift held; with Cmd/Ctrl held the selection is left alone (Cmd/Ctrl+Enter toggles).
pub fn focus_mode(modifiers: &egui::Modifiers) -> Option<SelectMode> {
    if modifiers.command {
        None
    } else if modifiers.shift {
        Some(SelectMode::Extend)
    } else {
        Some(SelectMode::Single)
    }
}

fn select(node: &NodeRef, mode: SelectMode) -> SidebarAction {
    SidebarAction::Select {
        node: node.clone(),
        mode,
    }
}

/// What a row did this frame. A plain click is `plain`; a Cmd/Ctrl or Shift click only changes
/// the selection; a right-click on an unselected row selects it alone (the menu then acts on it);
/// focus arriving by keyboard moves the selection; Copy and Paste keys act while a row has focus.
/// Without a `ctx` (no selection model) every click is plain.
pub fn node_action(
    ui: &egui::Ui,
    response: &egui::Response,
    ctx: Option<&TreeCtx>,
    node: &NodeRef,
    plain: impl FnOnce() -> SidebarAction,
) -> Option<SidebarAction> {
    let Some(ctx) = ctx else {
        return response.clicked().then(plain);
    };
    let modifiers = ui.input(|i| i.modifiers);
    if response.clicked() {
        return Some(match click_mode(&modifiers) {
            Some(mode) => select(node, mode),
            None => plain(),
        });
    }
    if response.secondary_clicked() {
        return (!ctx.is_selected(node)).then(|| select(node, SelectMode::Single));
    }
    if response.gained_focus() {
        let mode = focus_mode(&modifiers);
        let alone = ctx.state.selection.nodes() == std::slice::from_ref(node);
        return mode.filter(|_| !alone).map(|mode| select(node, mode));
    }
    key_action(ui, response, ctx, node)
}

/// Cmd/Ctrl+C and Cmd/Ctrl+V reach egui as copy and paste events, only while no text field holds
/// the focus; a focused row means the sidebar has it.
fn key_action(
    ui: &egui::Ui,
    response: &egui::Response,
    ctx: &TreeCtx,
    node: &NodeRef,
) -> Option<SidebarAction> {
    if !response.has_focus() {
        return None;
    }
    let events = ui.input(|i| i.events.clone());
    if events.iter().any(|e| matches!(e, egui::Event::Copy)) {
        return Some(SidebarAction::Copy);
    }
    let pasted = events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
    pasted.then(|| SidebarAction::Paste(paste_target_for(node, ctx.state.clipboard.as_ref())))
}

/// The Copy entry (disabled when the selection cannot be copied) and the Paste entry for
/// `target`, added to a row's context menu. Returns the one clicked.
pub fn copy_paste_entries(
    ui: &mut egui::Ui,
    ctx: Option<&TreeCtx>,
    target: PasteTarget,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let ctx = ctx?;
    ui.separator();
    let mut chosen = None;
    let copy = ctx.state.copy_entry();
    let Some((key, count)) = copy else {
        let button = ui.add_enabled(false, egui::Button::new(i18n.t("copy.menu.copy")));
        button.on_disabled_hover_text(i18n.t("copy.menu.copy_unavailable"));
        return paste_entry_button(ui, ctx.state.paste_entry(&target), target, i18n);
    };
    let label = i18n.t_with(key, &[("count", &count.to_string())]);
    if ui.button(label).clicked() {
        chosen = Some(SidebarAction::Copy);
        ui.close();
    }
    if let Some(action) = paste_entry_button(ui, ctx.state.paste_entry(&target), target, i18n) {
        chosen = Some(action);
    }
    chosen
}

/// The Paste entry alone (the connection root's menu has no Copy).
pub fn paste_entry_button(
    ui: &mut egui::Ui,
    entry: PasteEntry,
    target: PasteTarget,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let enabled = entry == PasteEntry::Enabled;
    let button = ui.add_enabled(enabled, egui::Button::new(i18n.t(PASTE_KEY)));
    match entry {
        PasteEntry::Disabled(reason) => {
            button.on_disabled_hover_text(i18n.t(reason));
            None
        }
        PasteEntry::Enabled if button.clicked() => {
            ui.close();
            Some(SidebarAction::Paste(target))
        }
        PasteEntry::Enabled => None,
    }
}

/// The Paste entry of a connection root's menu, and Cmd/Ctrl+V while the root name has focus.
/// Only the root-level paste (a database onto a connection) lands here.
pub fn root_action(
    response: &egui::Response,
    ui: &egui::Ui,
    state: &AppState,
    session: Uuid,
    i18n: &I18n,
) -> Option<SidebarAction> {
    let target = PasteTarget::Connection { session };
    let mut chosen = None;
    response.context_menu(|ui| {
        chosen = paste_entry_button(ui, state.paste_entry(&target), target.clone(), i18n);
    });
    let pasted = response.has_focus()
        && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
    chosen.or_else(|| pasted.then_some(SidebarAction::Paste(target)))
}

#[cfg(test)]
#[path = "sidebar_copy_tests.rs"]
mod tests;
