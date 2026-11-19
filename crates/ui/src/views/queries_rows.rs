//! The shapes of the Queries view, one block each: verdict as icon plus words (never colour
//! alone), where it was seen, how often it ran and missed, when it last ran, how slow it was, the
//! shape text and, for a miss, the suggested index. The shape and the suggestion each have a copy
//! button, and the suggestion a Create index... button that only opens the index builder.
use super::create_index::{CreateIndex, create_button};
use super::verdict_badge::chip_tone;
use crate::I18n;
use crate::theme::{card, chip, compact_text_button};
use crate::viewmodel::queries::{LastRun, LastRunBucket, last_run};
use crate::viewmodel::queries_status::{misses_key, row_title, runs_key};
use crate::viewmodel::verdict::kind_tone;
use crate::viewmodel::{QueriesAction, QueryRow};
use curlybrace_core::history::ShapeSource;
use eframe::egui;

/// Draws `rows` in order (the caller sorted them, misses first). `create` is the state of
/// each miss row's Create index... button. `now_secs` is the frame's time.
pub fn show_rows(
    ui: &mut egui::Ui,
    rows: &[QueryRow],
    create: CreateIndex,
    now_secs: i64,
    i18n: &I18n,
) -> Option<QueriesAction> {
    let total = rows.len();
    let mut action = None;
    for (index, row) in rows.iter().enumerate() {
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            let place = (index + 1, total);
            let shown = show_row(ui, row, place, (create, now_secs), i18n);
            action = keep_first(action.take(), shown);
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
    (create, now_secs): (CreateIndex, i64),
    i18n: &I18n,
) -> Option<QueriesAction> {
    ui.horizontal_wrapped(|ui| {
        let (icon, title_key) = row_title(row.latest);
        let verdict = format!("{icon} {}", i18n.t(title_key));
        chip(ui, chip_tone(kind_tone(row.latest)), &verdict);
        whole(
            ui,
            egui::RichText::new(i18n.t(source_key(row.source))).weak(),
        );
        whole(ui, count(runs_key(row.executions), row.executions, i18n));
        whole(ui, count(misses_key(row.misses), row.misses, i18n));
        // Typed observations carry no durations: "slowest 0 ms" would be a false statement.
        if row.source == ShapeSource::App {
            whole(ui, count("indexes.queries.slowest", row.max_millis, i18n));
        }
        if let Some(bucket) = row.last_run {
            whole(ui, last_run_text(bucket, now_secs, i18n));
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
    match row.suggestion.as_ref() {
        Some(command) => show_suggestion(ui, row, command, (create, &params), i18n),
        None => None,
    }
}

/// The suggested index with its Copy and Create index... buttons; the Create click, if any.
fn show_suggestion(
    ui: &mut egui::Ui,
    row: &QueryRow,
    command: &str,
    (create, params): (CreateIndex, &[(&str, &str)]),
    i18n: &I18n,
) -> Option<QueriesAction> {
    ui.label(i18n.t("query.verdict.suggestion"));
    let copy_label = i18n.t_with("indexes.queries.copy_suggestion", params);
    let create_label = i18n.t_with("indexes.queries.create_index_n", params);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(command).monospace());
        if compact_text_button(ui, &copy_label).clicked() {
            ui.ctx().copy_text(command.to_string());
        }
        create_button(ui, create, Some(&create_label), i18n).then(|| QueriesAction::CreateIndex {
            hash: row.hash.clone(),
            source: row.source,
        })
    })
    .inner
}

/// The action already chosen this frame, else the new one: the first row clicked wins and a
/// later one is ignored.
fn keep_first(chosen: Option<QueriesAction>, next: Option<QueriesAction>) -> Option<QueriesAction> {
    chosen.or(next)
}

/// A label that moves to the next line whole instead of splitting in the middle.
fn whole(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
}

/// "Last run 20 min ago", or "Last run within the last hour" when the newest bucket says no more.
fn last_run_text(bucket: LastRunBucket, now_secs: i64, i18n: &I18n) -> String {
    match last_run(bucket, now_secs) {
        LastRun::Ago(age) => {
            let when = i18n.t_with(age.key(), &[("n", &age.count().to_string())]);
            i18n.t_with("indexes.queries.last_run", &[("when", &when)])
        }
        LastRun::Within(width) => i18n.t(width.within_key()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::history::ShapeSource;

    fn create(hash: &str) -> Option<QueriesAction> {
        Some(QueriesAction::CreateIndex {
            hash: hash.into(),
            source: ShapeSource::App,
        })
    }

    #[test]
    fn the_first_row_clicked_in_a_frame_wins() {
        assert_eq!(keep_first(create("a"), create("b")), create("a"));
        assert_eq!(keep_first(None, create("b")), create("b"));
        assert_eq!(keep_first(create("a"), None), create("a"));
        assert_eq!(keep_first(None, None), None);
    }
}
