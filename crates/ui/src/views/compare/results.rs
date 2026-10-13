//! Category tabs with counts, the key table of the current page and the pager.
use super::detail::{forget_cache, show_detail};
use super::select::{header_checkbox, row_checkbox, show_selection_bar};
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{
    CompareState, ResultsView, SelectionBar, category_key, key_label, page_ticked, visible_ticks,
};
use curlybrace_core::compare::protocol::RowView;
use curlybrace_core::compare::results::Category;
use eframe::egui;

const TABLE_MAX_HEIGHT: f32 = 280.0;

const TAB_ORDER: [Category; 3] = [
    Category::Different,
    Category::OnlyInSource,
    Category::OnlyInTarget,
];

/// The screen of one pair's results: its names, Back to overview, then the results table and
/// the open detail. Nothing is drawn (and the detail cache is dropped) when no pair is open.
pub fn show_pair_results(
    ui: &mut egui::Ui,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    let Some(results) = state.open_results() else {
        forget_cache(ui);
        return None;
    };
    let back = show_heading(ui, state, i18n);
    let table = show_results(ui, state, results, i18n);
    let detail = match &state.detail {
        Some(d) => {
            ui.separator();
            show_detail(ui, d, &state.detail_buttons(), i18n)
        }
        None => {
            forget_cache(ui);
            None
        }
    };
    back.or(table).or(detail)
}

fn show_heading(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if ui
            .button(i18n.t("compare.results.back_to_overview"))
            .clicked()
        {
            action = Some(CompareAction::BackToOverview);
        }
        if let Some((source, target)) = state.open_pair_labels() {
            let pair = i18n.t_with(
                "compare.pairs.pair_label",
                &[("source", &source), ("target", &target)],
            );
            ui.heading(pair);
        }
        show_pair_status(ui, state, i18n);
    });
    if state.open_pair.is_some_and(|p| state.pair_is_applied(p)) {
        let color = ui.visuals().warn_fg_color;
        ui.colored_label(color, i18n.t("compare.apply.applied_note"));
    }
    action
}

/// How many documents are queued for the open pair; nothing when none are.
fn show_pair_status(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) {
    let queued = state.open_pair.map_or(0, |p| state.pending_documents(p));
    if queued > 0 {
        let count = queued.to_string();
        ui.label(i18n.t_with("compare.apply.pending_badge", &[("count", &count)]));
    }
}

pub fn show_results(
    ui: &mut egui::Ui,
    state: &CompareState,
    results: &ResultsView,
    i18n: &I18n,
) -> Option<CompareAction> {
    let tabs = show_tabs(ui, results, i18n);
    let identical = results.counts.identical.to_string();
    ui.label(i18n.t_with("compare.results.identical_count", &[("count", &identical)]));
    if results.is_clean() {
        ui.label(i18n.t("compare.results.identical"));
        return tabs;
    }
    if results.total() == 0 {
        ui.label(i18n.t("compare.results.empty_category"));
        return tabs;
    }
    let pager = show_pager(ui, results, i18n);
    let bar = show_selection_bar(ui, &SelectionBar::of(state, results), i18n);
    let row = show_table(ui, state, results, i18n);
    tabs.or(pager).or(bar).or(row)
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
    state: &CompareState,
    results: &ResultsView,
    i18n: &I18n,
) -> Option<CompareAction> {
    if results.load_failed {
        return show_load_failed(ui, i18n);
    }
    if results.is_loading() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(i18n.t("compare.results.loading"));
        });
        return None;
    }
    // One membership lookup per visible row; the selection itself is never walked.
    let ticks = visible_ticks(&state.selection, results);
    let enabled = state.tick_block(results.pair).is_none();
    let header = show_table_header(ui, &ticks, enabled, i18n);
    let selected = state.detail.as_ref().map(|d| &d.key);
    let mut action = None;
    egui::ScrollArea::vertical()
        .id_salt("compare_rows")
        .max_height(TABLE_MAX_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, row) in results.rows.iter().enumerate() {
                let id = (category_key(results.category), results.offset + index);
                let line = RowLine {
                    row,
                    ticked: ticks[index],
                    enabled,
                    current: selected == Some(&row.key),
                };
                let clicked = ui.push_id(id, |ui| show_row(ui, &line, i18n)).inner;
                action = clicked.or(action.take());
            }
        });
    header.or(action)
}

/// The "Key" column title with the checkbox that ticks the loaded page.
fn show_table_header(
    ui: &mut egui::Ui,
    ticks: &[bool],
    enabled: bool,
    i18n: &I18n,
) -> Option<CompareAction> {
    let all = page_ticked(ticks);
    ui.horizontal(|ui| {
        let clicked = header_checkbox(ui, all, enabled && !ticks.is_empty(), i18n);
        ui.strong(i18n.t("compare.results.key_column"));
        clicked.then_some(if all {
            CompareAction::ClearSelection
        } else {
            CompareAction::SelectPage
        })
    })
    .inner
}

/// The page could not be loaded: say so and offer to ask again.
fn show_load_failed(ui: &mut egui::Ui, i18n: &I18n) -> Option<CompareAction> {
    ui.horizontal(|ui| {
        let color = ui.visuals().error_fg_color;
        ui.colored_label(color, i18n.t("compare.results.load_failed"));
        let retry = ui.button(i18n.t("compare.results.retry"));
        retry.clicked().then_some(CompareAction::RetryRows)
    })
    .inner
}

/// What one table line shows.
struct RowLine<'a> {
    row: &'a RowView,
    ticked: bool,
    /// Ticking is open for this pair.
    enabled: bool,
    /// The row whose detail is open.
    current: bool,
}

/// One row: its checkbox, then its key as a selectable monospace line (click opens the detail).
fn show_row(ui: &mut egui::Ui, line: &RowLine<'_>, i18n: &I18n) -> Option<CompareAction> {
    let label = key_label(&line.row.key);
    ui.horizontal(|ui| {
        let toggled = row_checkbox(ui, line.ticked, line.enabled, &label);
        let text = egui::RichText::new(&label).monospace();
        let response = ui.selectable_label(line.current, text);
        let opened = response
            .on_hover_text(i18n.t("compare.results.open_hint"))
            .clicked();
        if toggled {
            Some(CompareAction::ToggleRow(line.row.key.clone()))
        } else {
            opened.then(|| CompareAction::Select(line.row.key.clone()))
        }
    })
    .inner
}
