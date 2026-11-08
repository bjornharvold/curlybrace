//! The shapes of the Queries view, one block each: verdict as icon plus words (never colour
//! alone), where it was seen, how often it ran and missed, how slow it was, the shape text and,
//! for a miss, the suggested index. The shape and the suggestion each have a copy button.
use super::verdict_badge::chip_tone;
use crate::I18n;
use crate::theme::{card, chip, compact_text_button};
use crate::viewmodel::QueryRow;
use crate::viewmodel::queries_status::{misses_key, row_title, runs_key};
use crate::viewmodel::verdict::kind_tone;
use curlybrace_core::history::ShapeSource;
use eframe::egui;

/// Draws `rows` in order (the caller sorted them, misses first).
pub fn show_rows(ui: &mut egui::Ui, rows: &[QueryRow], i18n: &I18n) {
    let total = rows.len();
    for (index, row) in rows.iter().enumerate() {
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            show_row(ui, row, (index + 1, total), i18n);
        });
    }
}

/// `place` is the 1-based position of the row and the number of rows: the copy buttons name it,
/// so each is told apart by assistive technology.
fn show_row(ui: &mut egui::Ui, row: &QueryRow, place: (usize, usize), i18n: &I18n) {
    ui.horizontal_wrapped(|ui| {
        let (icon, title_key) = row_title(row.latest);
        let verdict = format!("{icon} {}", i18n.t(title_key));
        chip(ui, chip_tone(kind_tone(row.latest)), &verdict);
        ui.weak(i18n.t(source_key(row.source)));
        ui.label(count(runs_key(row.executions), row.executions, i18n));
        ui.label(count(misses_key(row.misses), row.misses, i18n));
        // Typed observations carry no durations: "slowest 0 ms" would be a false statement.
        if row.source == ShapeSource::App {
            ui.label(count("indexes.queries.slowest", row.max_millis, i18n));
        }
    });
    if row.worst != row.latest {
        let (_, worst_key) = row_title(row.worst);
        let worst = i18n.t(worst_key);
        ui.weak(i18n.t_with("indexes.queries.worst_in_range", &[("verdict", &worst)]));
    }
    let params = [
        ("position", place.0.to_string()),
        ("total", place.1.to_string()),
    ];
    let params: Vec<(&str, &str)> = params.iter().map(|(n, v)| (*n, v.as_str())).collect();
    let shape_label = i18n.t_with("indexes.queries.copy_shape", &params);
    copyable(ui, &row.shape_json, &shape_label);
    if let Some(command) = &row.suggestion {
        ui.label(i18n.t("query.verdict.suggestion"));
        let label = i18n.t_with("indexes.queries.copy_suggestion", &params);
        copyable(ui, command, &label);
    }
}

/// `text` in monospace with a copy button named `copy_label` (already translated).
pub(super) fn copyable(ui: &mut egui::Ui, text: &str, copy_label: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(text).monospace());
        if compact_text_button(ui, copy_label).clicked() {
            ui.ctx().copy_text(text.to_string());
        }
    });
}

fn count(key: &str, n: u64, i18n: &I18n) -> String {
    i18n.t_with(key, &[("n", &n.to_string())])
}

fn source_key(source: ShapeSource) -> &'static str {
    match source {
        ShapeSource::App => "indexes.queries.source_app",
        ShapeSource::Typed => "indexes.queries.source_typed",
    }
}
