//! The Run button of a tab whose connection was lost (#159): the tab keeps its query and waits.
use crate::I18n;
use crate::theme::tonal_button;
use eframe::egui::{self, WidgetInfo, WidgetType};

/// A button that sends the dormant tab's find; true when it was pressed. Its name says what
/// pressing it does (the find bar's own Find button has the plain name).
pub fn show_dormant_run(ui: &mut egui::Ui, i18n: &I18n) -> bool {
    let name = i18n.t("tabs.dormant_run");
    let button = tonal_button(ui, i18n.t("findbar.run"));
    button.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &name));
    button.clicked()
}
