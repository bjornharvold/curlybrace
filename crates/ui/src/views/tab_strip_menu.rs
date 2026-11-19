//! The two menus of the tab strip: a tab's context menu and the "All tabs" list (#159).
use crate::I18n;
use crate::app::tab_actions::TabAction;
use crate::app::tabs::TabId;
use crate::theme::{icons, text_button};
use crate::views::connection_badge::show_badge;
use crate::views::tab_strip::StripView;
use eframe::egui::{self, Ui, WidgetInfo, WidgetType};

/// The entries of the context menu of `tab`, the `at`-th of `count` shown. Closing the menu is
/// left to egui (a click on an entry closes it).
pub fn tab_context_menu(
    ui: &mut Ui,
    (tab, at, count): (TabId, usize, usize),
    can_reopen: bool,
    i18n: &I18n,
) -> Option<TabAction> {
    let mut chosen = None;
    let mut entry = |ui: &mut Ui, enabled: bool, key: &str, action: TabAction| {
        let button = egui::Button::new(i18n.t(key));
        if ui.add_enabled(enabled, button).clicked() {
            chosen = Some(action);
            ui.close();
        }
    };
    entry(ui, true, "tabs.duplicate", TabAction::Duplicate(tab));
    ui.separator();
    entry(ui, true, "tabs.close", TabAction::Close(tab));
    entry(
        ui,
        count > 1,
        "tabs.close_others",
        TabAction::CloseOthers(tab),
    );
    entry(
        ui,
        at + 1 < count,
        "tabs.close_right",
        TabAction::CloseRight(tab),
    );
    ui.separator();
    let left = TabAction::Move {
        tab,
        to: at.saturating_sub(1),
    };
    entry(ui, at > 0, "tabs.move_left", left);
    let right = TabAction::Move { tab, to: at + 1 };
    entry(ui, at + 1 < count, "tabs.move_right", right);
    ui.separator();
    entry(
        ui,
        can_reopen,
        "tabs.reopen_closed",
        TabAction::ReopenClosed,
    );
    chosen
}

/// The chevron button pinned at the right of the strip, and the list of every tab it opens (with
/// each tab's badge); choosing one shows it.
pub fn all_tabs_menu(ui: &mut Ui, view: &StripView<'_>, i18n: &I18n) -> Option<TabAction> {
    let name = i18n.t("tabs.all_tabs");
    let glyph = icons::inline_icon(ui, icons::CHEVRON_DOWN);
    let button = text_button(ui, glyph).on_hover_text(&name);
    button.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name.clone()));
    let mut chosen = None;
    egui::Popup::menu(&button).show(|ui| {
        for (tab, label) in view.labels {
            ui.horizontal(|ui| {
                show_badge(ui, &label.badge, i18n);
                let current = view.active == Some(*tab);
                // 24 pt tall, WCAG 2.2's smallest target.
                let entry = ui.add(
                    egui::Button::selectable(current, &label.title).min_size(egui::vec2(0.0, 24.0)),
                );
                // Named by its place too: two tabs on one collection are told apart.
                entry.widget_info(|| {
                    WidgetInfo::selected(WidgetType::Button, true, current, &label.menu_name)
                });
                if entry.on_hover_text(&label.tooltip).clicked() {
                    chosen = Some(TabAction::Activate(*tab));
                    ui.close();
                }
            });
        }
    });
    chosen
}
