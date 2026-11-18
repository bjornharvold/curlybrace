//! The shapes of the Queries view, one block each: verdict as icon plus words (never colour
//! alone), where it was seen, how often it ran and missed, when it last ran, how slow it was, the
//! shape text and, for a miss, the suggested index. The shape and the suggestion each have a copy
//! button, and the suggestion a Create index... button that only opens the index builder.
use super::findbar::described;
use super::verdict_badge::chip_tone;
use crate::I18n;
use crate::theme::{card, chip, compact_text_button, gate, tonal_button};
use crate::viewmodel::index_detail::Age;
use crate::viewmodel::queries_status::{misses_key, row_title, runs_key};
use crate::viewmodel::verdict::kind_tone;
use crate::viewmodel::{QueriesAction, QueryRow};
use curlybrace_core::history::ShapeSource;
use eframe::egui;

/// Draws `rows` in order (the caller sorted them, misses first). `create_off` is why Create
/// index... is off (its `en.json` key), or `None` while it works. `now_secs` is the frame's time.
pub fn show_rows(
    ui: &mut egui::Ui,
    rows: &[QueryRow],
    create_off: Option<&'static str>,
    now_secs: i64,
    i18n: &I18n,
) -> Option<QueriesAction> {
    let total = rows.len();
    let mut action = None;
    for (index, row) in rows.iter().enumerate() {
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            let place = (index + 1, total);
            let shown = show_row(ui, row, place, (create_off, now_secs), i18n);
            action = shown.or(action.take());
        });
    }
    action
}

/// `place` is the 1-based position of the row and the number of rows: the buttons name it, so
/// each is told apart by assistive technology.
fn show_row(
    ui: &mut egui::Ui,
    row: &QueryRow,
    place: (usize, usize),
    (create_off, now_secs): (Option<&'static str>, i64),
    i18n: &I18n,
) -> Option<QueriesAction> {
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
        if let Some(start) = row.last_run_secs {
            ui.label(last_run(start, now_secs, i18n));
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
    let command = row.suggestion.as_ref()?;
    ui.label(i18n.t("query.verdict.suggestion"));
    let label = i18n.t_with("indexes.queries.copy_suggestion", &params);
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(command).monospace());
        if compact_text_button(ui, &label).clicked() {
            ui.ctx().copy_text(command.clone());
        }
        let create_label = i18n.t_with("indexes.queries.create_index_n", &params);
        if create_button(ui, create_off, &create_label, i18n) {
            action = Some(QueriesAction::CreateIndex {
                hash: row.hash.clone(),
            });
        }
    });
    action
}

/// The Create index... button: the drawn text is short, the accessible name says which query it is
/// for. Off, it keeps its place and says why as hover text and accessible description.
fn create_button(
    ui: &mut egui::Ui,
    off: Option<&'static str>,
    accessible_name: &str,
    i18n: &I18n,
) -> bool {
    let response = gate(ui, off.is_none(), |ui| {
        tonal_button(ui, i18n.t("indexes.queries.create_index"))
    });
    let name = accessible_name.to_string();
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(name));
    match off {
        Some(key) => {
            described(ui, response, i18n.t(key));
            false
        }
        None => response.clicked(),
    }
}

/// "Last run 20 min ago", from the start of the newest bucket that ran the shape.
fn last_run(start_secs: i64, now_secs: i64, i18n: &I18n) -> String {
    let age = Age::of(now_secs - start_secs);
    let when = i18n.t_with(age.key(), &[("n", &age.count().to_string())]);
    i18n.t_with("indexes.queries.last_run", &[("when", &when)])
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
