//! The run overview: one row per pair that has run (status with icon and label, the counts or
//! progress or error, and the row's actions), the totals, and Cancel while a run is in flight.
//! Rows are drawn with `show_rows`; the totals are summed once per change of the overview.
use super::duplicates::show_duplicates;
use super::memo::memo;
use super::table::{
    Col, MAX_ROWS_HEIGHT, ROW_HEIGHT, cell, header, status_text, truncated, widths,
};
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{
    CompareState, OverviewRow, RowCounts, RowDetail, RunOverview, Totals,
};
use eframe::egui;

/// Pair, status, counts and the row's buttons.
const COLUMNS: [Col; 4] = [
    Col::Flex(3.0),
    Col::Fixed(120.0),
    Col::Flex(3.5),
    Col::Fixed(300.0),
];

pub fn show_overview(
    ui: &mut egui::Ui,
    state: &CompareState,
    i18n: &I18n,
) -> Option<CompareAction> {
    if state.overview.rows.is_empty() {
        return None;
    }
    ui.separator();
    let cancel = show_title(ui, state, i18n);
    let widths = widths(ui, &COLUMNS);
    header(ui, &widths, &titles(i18n));
    let rows = show_rows(ui, state, &widths, i18n);
    show_totals(ui, &state.overview, &widths, i18n);
    let duplicates = show_duplicates(ui, state, i18n);
    cancel.or(rows).or(duplicates)
}

fn titles(i18n: &I18n) -> Vec<String> {
    vec![
        i18n.t("compare.overview.column.pair"),
        i18n.t("compare.overview.column.status"),
        i18n.t("compare.overview.column.counts"),
        String::new(),
    ]
}

fn show_title(ui: &mut egui::Ui, state: &CompareState, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.strong(i18n.t("compare.overview.title"));
        if !state.is_running() {
            return;
        }
        let cancel = egui::Button::new(i18n.t("compare.overview.cancel"));
        if ui.add_enabled(state.can_cancel(), cancel).clicked() {
            action = Some(CompareAction::Cancel);
        }
        if state.cancel_requested() {
            ui.spinner();
            ui.label(i18n.t("compare.overview.cancelling"));
        }
    });
    action
}

fn show_rows(
    ui: &mut egui::Ui,
    state: &CompareState,
    widths: &[f32],
    i18n: &I18n,
) -> Option<CompareAction> {
    egui::ScrollArea::vertical()
        .id_salt("compare_overview_rows")
        .max_height(MAX_ROWS_HEIGHT)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, state.overview.rows.len(), |ui, range| {
            let mut action = None;
            for index in range {
                let Some(row) = state.overview_row(index) else {
                    ui.add_space(ROW_HEIGHT);
                    continue;
                };
                let shown = ui
                    .push_id(row.entry, |ui| show_row(ui, widths, &row, i18n))
                    .inner;
                action = shown.or(action);
            }
            action
        })
        .inner
}

fn show_row(ui: &mut egui::Ui, w: &[f32], row: &OverviewRow, i18n: &I18n) -> Option<CompareAction> {
    let pair = i18n.t_with(
        "compare.pairs.pair_label",
        &[("source", &row.source), ("target", &row.target)],
    );
    ui.horizontal(|ui| {
        cell(ui, w[0], |ui| truncated(ui, &pair));
        cell(ui, w[1], |ui| {
            let label = i18n.t(row.status_key);
            truncated(ui, &status_text(row.icon, &label));
        });
        cell(ui, w[2], |ui| show_detail(ui, &row.detail, i18n));
        cell(ui, w[3], |ui| row_buttons(ui, row, i18n))
    })
    .inner
}

fn show_detail(ui: &mut egui::Ui, detail: &RowDetail, i18n: &I18n) {
    match detail {
        RowDetail::None => {}
        RowDetail::Counts(counts) => truncated(ui, &counts_text(counts, i18n)),
        RowDetail::Progress {
            scanned_source,
            scanned_target,
            differences,
        } => {
            let text = i18n.t_with(
                "compare.overview.progress",
                &[
                    ("source", &scanned_source.to_string()),
                    ("target", &scanned_target.to_string()),
                    ("differences", &differences.to_string()),
                ],
            );
            truncated(ui, &text);
        }
        RowDetail::Error(error) => {
            let text = i18n.error(error);
            let color = ui.visuals().error_fg_color;
            ui.add(egui::Label::new(egui::RichText::new(&text).color(color)).truncate())
                .on_hover_text(text);
        }
    }
}

fn counts_text(counts: &RowCounts, i18n: &I18n) -> String {
    i18n.t_with(
        "compare.overview.counts",
        &[
            ("different", &counts.different.to_string()),
            ("only_in_source", &counts.only_in_source.to_string()),
            ("only_in_target", &counts.only_in_target.to_string()),
            ("identical", &counts.identical.to_string()),
        ],
    )
}

fn row_buttons(ui: &mut egui::Ui, row: &OverviewRow, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    let mut button = |ui: &mut egui::Ui, key: &str, on: bool, make: CompareAction| {
        if ui.add_enabled(on, egui::Button::new(i18n.t(key))).clicked() {
            action = Some(make);
        }
    };
    button(
        ui,
        "compare.overview.open",
        row.can_open,
        CompareAction::OpenPair(row.entry),
    );
    button(
        ui,
        "compare.overview.discard",
        row.can_discard,
        CompareAction::DiscardPair(row.entry),
    );
    button(
        ui,
        "compare.overview.rerun",
        row.can_rerun,
        CompareAction::Rerun(row.entry),
    );
    if row.offers_duplicates {
        let find = CompareAction::FindDuplicates(row.entry);
        button(ui, "compare.duplicates.find", row.can_find_duplicates, find);
    }
    action
}

/// The totals line. Summed over every row once per change of the overview, not once per frame.
fn show_totals(ui: &mut egui::Ui, overview: &RunOverview, w: &[f32], i18n: &I18n) {
    let totals = memo(
        ui,
        "overview_totals",
        |stored: &RunOverview| stored == overview,
        || (overview.clone(), overview.totals()),
    );
    let Totals {
        completed,
        failed,
        cancelled,
        ..
    } = *totals;
    let status = i18n.t_with(
        "compare.overview.totals_status",
        &[
            ("completed", &completed.to_string()),
            ("failed", &failed.to_string()),
            ("cancelled", &cancelled.to_string()),
        ],
    );
    let counts = counts_text(&RowCounts::from(&*totals), i18n);
    ui.horizontal(|ui| {
        cell(ui, w[0], |ui| {
            ui.strong(i18n.t("compare.overview.totals"));
        });
        cell(ui, w[1], |ui| truncated(ui, &status));
        cell(ui, w[2], |ui| truncated(ui, &counts));
    });
}
