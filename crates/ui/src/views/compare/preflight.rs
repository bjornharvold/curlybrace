//! The preflight strip: Check (the queue over the included pairs), Run and Run passing, and one
//! verdict row per included pair. What is enabled is `CompareState::gates`; what a verdict says
//! is `PairRow::status`.
use super::table::{Col, MAX_ROWS_HEIGHT, ROW_HEIGHT, cell, entry_status_label, truncated, widths};
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{CompareState, EntrySizes, Gates, ListView, PairRow};
use eframe::egui;

/// Pair, verdict and what the check found (or what to do about it).
const COLUMNS: [Col; 3] = [Col::Flex(2.2), Col::Flex(2.4), Col::Flex(4.4)];

pub fn show_preflight(
    ui: &mut egui::Ui,
    state: &CompareState,
    gates: &Gates,
    list: &ListView,
    i18n: &I18n,
) -> Option<CompareAction> {
    let buttons = show_buttons(ui, state, gates, i18n);
    show_verdicts(ui, state, list, i18n);
    buttons
}

fn show_buttons(
    ui: &mut egui::Ui,
    state: &CompareState,
    gates: &Gates,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        // While a check is in flight another cannot start, and an edit meanwhile leaves the
        // answer stale: the button says so instead of silently doing nothing.
        let label = if state.check_in_flight() {
            "compare.preflight.checking"
        } else {
            "compare.preflight.check"
        };
        let check = egui::Button::new(i18n.t(label));
        if ui.add_enabled(gates.check, check).clicked() {
            action = Some(CompareAction::RunCheck);
        }
        if state.check_in_flight() {
            ui.spinner();
        }
        let run = egui::Button::new(i18n.t("compare.preflight.run"));
        if ui.add_enabled(gates.start, run).clicked() {
            action = Some(CompareAction::Start);
        }
        if gates.some_not_passing {
            let passing = egui::Button::new(i18n.t("compare.preflight.run_passing"));
            if ui.add_enabled(gates.start_passing, passing).clicked() {
                action = Some(CompareAction::StartPassing);
            }
        }
    });
    action
}

fn show_verdicts(ui: &mut egui::Ui, state: &CompareState, list: &ListView, i18n: &I18n) {
    if list.included.is_empty() {
        return;
    }
    let widths = widths(ui, &COLUMNS);
    egui::ScrollArea::vertical()
        .id_salt("compare_verdict_rows")
        .max_height(MAX_ROWS_HEIGHT)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, list.included.len(), |ui, range| {
            for index in &list.included[range] {
                if let Some(row) = state.pair_row(*index, &list.digest) {
                    ui.push_id(row.entry, |ui| show_verdict(ui, &widths, &row, i18n));
                }
            }
        });
}

fn show_verdict(ui: &mut egui::Ui, w: &[f32], row: &PairRow, i18n: &I18n) {
    let pair = i18n.t_with(
        "compare.pairs.pair_label",
        &[("source", &row.source), ("target", &row.target)],
    );
    ui.horizontal(|ui| {
        cell(ui, w[0], |ui| truncated(ui, &pair));
        cell(ui, w[1], |ui| {
            truncated(ui, &entry_status_label(&row.status, i18n));
        });
        cell(ui, w[2], |ui| show_finding(ui, row, i18n));
    });
}

/// What to do about a missing index, else the sizes the check found. Both fit one line, so the
/// advice (when there is some) is shown and the sizes ride on its hover text.
fn show_finding(ui: &mut egui::Ui, row: &PairRow, i18n: &I18n) {
    let sizes = row.sizes.as_ref().map(|s| sizes_text(s, i18n));
    let Some(side_key) = row.status.side_key() else {
        if let Some(text) = sizes {
            truncated(ui, &text);
        }
        return;
    };
    let side = i18n.t(side_key);
    let next = i18n.t_with("compare.preflight.needs_index", &[("side", &side)]);
    let hover = sizes.map_or(next.clone(), |s| format!("{s}\n{next}"));
    let color = ui.visuals().error_fg_color;
    ui.add(egui::Label::new(egui::RichText::new(&next).color(color)).truncate())
        .on_hover_text(hover);
}

/// Per side: document count and whether the key is indexed.
fn sizes_text(sizes: &EntrySizes, i18n: &I18n) -> String {
    i18n.t_with(
        "compare.preflight.entry_sizes",
        &[
            ("source_side", &i18n.t("compare.setup.source_heading")),
            ("source_count", &sizes.source_count.to_string()),
            ("source_index", &i18n.t(sizes.source_index_key)),
            ("target_side", &i18n.t("compare.setup.target_heading")),
            ("target_count", &sizes.target_count.to_string()),
            ("target_index", &i18n.t(sizes.target_index_key)),
        ],
    )
}
