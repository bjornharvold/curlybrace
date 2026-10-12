//! Category tabs with counts, the key table of the current page and the pager.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{ResultsView, category_key, key_label};
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::RowView;
use curlybrace_core::compare::results::Category;
use eframe::egui;

const TABLE_MAX_HEIGHT: f32 = 280.0;

const TAB_ORDER: [Category; 3] = [
    Category::Different,
    Category::OnlyInSource,
    Category::OnlyInTarget,
];

pub fn show_results(
    ui: &mut egui::Ui,
    results: &ResultsView,
    selected: Option<&KeyTuple>,
    i18n: &I18n,
) -> Option<CompareAction> {
    let tabs = show_tabs(ui, results, i18n);
    if results.is_clean() {
        ui.label(i18n.t("compare.results.identical"));
        return tabs;
    }
    if results.total() == 0 {
        ui.label(i18n.t("compare.results.empty_category"));
        return tabs;
    }
    let pager = show_pager(ui, results, i18n);
    let row = show_table(ui, results, selected, i18n);
    tabs.or(pager).or(row)
}

fn show_tabs(ui: &mut egui::Ui, results: &ResultsView, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        for category in TAB_ORDER {
            let name = i18n.t(category_key(category));
            let count = results.counts.of(category).to_string();
            let title = i18n.t_with("compare.results.tab", &[("name", &name), ("count", &count)]);
            let current = category == results.category;
            let tab = ui.push_id(category_key(category), |ui| {
                ui.selectable_label(current, title)
            });
            if tab.inner.clicked() {
                action = Some(CompareAction::SelectCategory(category));
            }
        }
    });
    action
}

fn show_pager(ui: &mut egui::Ui, results: &ResultsView, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let prev = egui::Button::new(i18n.t("compare.results.prev"));
        if ui.add_enabled(results.has_prev(), prev).clicked() {
            action = Some(CompareAction::Page(-1));
        }
        let next = egui::Button::new(i18n.t("compare.results.next"));
        if ui.add_enabled(results.has_next(), next).clicked() {
            action = Some(CompareAction::Page(1));
        }
        let (first, last) = results.shown_range();
        let range = i18n.t_with(
            "compare.results.range",
            &[
                ("first", &first.to_string()),
                ("last", &last.to_string()),
                ("total", &results.total().to_string()),
            ],
        );
        ui.label(range);
    });
    action
}

fn show_table(
    ui: &mut egui::Ui,
    results: &ResultsView,
    selected: Option<&KeyTuple>,
    i18n: &I18n,
) -> Option<CompareAction> {
    if results.is_loading() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(i18n.t("compare.results.loading"));
        });
        return None;
    }
    ui.strong(i18n.t("compare.results.key_column"));
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("compare_rows")
        .max_height(TABLE_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, row) in results.rows.iter().enumerate() {
                let current = selected == Some(&row.key);
                let id = (category_key(results.category), results.offset + index);
                let clicked = ui.push_id(id, |ui| show_row(ui, row, current, i18n)).inner;
                if clicked {
                    action = Some(CompareAction::Select(row.key.clone()));
                }
            }
        });
    action
}

/// One key as a selectable monospace line; true when it was clicked.
fn show_row(ui: &mut egui::Ui, row: &RowView, current: bool, i18n: &I18n) -> bool {
    let text = egui::RichText::new(key_label(&row.key)).monospace();
    let response = ui.selectable_label(current, text);
    response
        .on_hover_text(i18n.t("compare.results.open_hint"))
        .clicked()
}
