//! Opening a collection in a new tab from its sidebar row: a middle click, or Alt+Enter while the
//! row has the keyboard (Cmd+click and Shift+click are the multi-select) (#159).
use crate::viewmodel::SidebarAction;
use curlybrace_core::store::Namespace;
use eframe::egui::{Key, Modifiers, Response, Ui};

/// "Open in new tab" for the row `db.name` when the person asked for it this frame.
pub fn new_tab_request(
    ui: &Ui,
    response: &Response,
    db: &str,
    name: &str,
) -> Option<SidebarAction> {
    let alt_enter =
        response.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::Enter));
    (response.middle_clicked() || alt_enter)
        .then(|| SidebarAction::OpenInNewTab(Namespace::new(db, name)))
}
