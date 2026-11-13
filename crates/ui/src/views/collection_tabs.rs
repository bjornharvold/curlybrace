//! The switch between the tabs of a collection (Documents | Indexes | Aggregate), drawn as Material primary
//! tabs.
use crate::I18n;
use crate::theme::tabs_inline;
use crate::viewmodel::CollectionTab;
use eframe::egui;

const TABS: [CollectionTab; 3] = [
    CollectionTab::Documents,
    CollectionTab::Indexes,
    CollectionTab::Aggregate,
];

/// Draws the tabs at their natural width, so they can follow the connection header on its row;
/// returns the one the person picked, if it is not the current one.
pub fn show_collection_tabs(
    ui: &mut egui::Ui,
    current: CollectionTab,
    i18n: &I18n,
) -> Option<CollectionTab> {
    let items: Vec<(CollectionTab, String)> = TABS
        .into_iter()
        .map(|tab| (tab, i18n.t(tab.label_key())))
        .collect();
    tabs_inline(ui, &items, current)
}
