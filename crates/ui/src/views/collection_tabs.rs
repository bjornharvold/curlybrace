//! The switch between the tabs of a collection (Documents | Indexes), drawn as Material primary
//! tabs.
use crate::I18n;
use crate::theme::tabs;
use crate::viewmodel::CollectionTab;
use eframe::egui;

const TABS: [CollectionTab; 2] = [CollectionTab::Documents, CollectionTab::Indexes];

/// Draws the tabs; returns the one the person picked, if it is not the current one.
pub fn show_collection_tabs(
    ui: &mut egui::Ui,
    current: CollectionTab,
    i18n: &I18n,
) -> Option<CollectionTab> {
    let items: Vec<(CollectionTab, String)> = TABS
        .into_iter()
        .map(|tab| (tab, i18n.t(tab.label_key())))
        .collect();
    tabs(ui, &items, current)
}
