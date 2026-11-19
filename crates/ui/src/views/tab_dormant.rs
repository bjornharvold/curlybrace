//! The Run button of a tab whose connection was lost (#159): the tab keeps its query and waits.
use crate::I18n;
use crate::theme::filled_button;
use eframe::egui;

/// One button that sends the dormant tab's find, named by what it does; true when pressed.
pub fn show_dormant_run(ui: &mut egui::Ui, i18n: &I18n) -> bool {
    filled_button(ui, i18n.t("tabs.dormant_run")).clicked()
}
