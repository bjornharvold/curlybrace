//! The switch between the tabs of a collection (Documents | Indexes).
use crate::I18n;
use crate::viewmodel::CollectionTab;
use eframe::egui;

const TABS: [CollectionTab; 2] = [CollectionTab::Documents, CollectionTab::Indexes];

/// Draws the tabs; returns the one the person picked, if it is not the current one.
pub fn show_collection_tabs(
    ui: &mut egui::Ui,
    current: CollectionTab,
    i18n: &I18n,
) -> Option<CollectionTab> {
    let mut picked = None;
    ui.horizontal(|ui| {
        for tab in TABS {
            let selected = tab == current;
            if ui
                .selectable_label(selected, i18n.t(tab.label_key()))
                .clicked()
                && !selected
            {
                picked = Some(tab);
            }
        }
    });
    picked
}
