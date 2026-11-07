//! The shapes of the Queries view, one block each: verdict as icon plus words (never colour
//! alone), where it was seen, how often it ran and missed, how slow it was, the shape text and,
//! for a miss, the suggested index. The shape and the suggestion each have a copy button.
use crate::I18n;
use crate::viewmodel::QueryRow;
use crate::viewmodel::verdict::kind_presentation;
use curlybrace_core::history::{ShapeSource, is_miss};
use eframe::egui;

/// Draws `rows` in order (the caller sorted them, misses first).
pub fn show_rows(ui: &mut egui::Ui, rows: &[QueryRow], i18n: &I18n) {
    for row in rows {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            show_row(ui, row, i18n);
        });
    }
}

fn show_row(ui: &mut egui::Ui, row: &QueryRow, i18n: &I18n) {
    ui.horizontal_wrapped(|ui| {
        let (icon, title_key) = kind_presentation(row.worst);
        let title = egui::RichText::new(format!("{icon} {}", i18n.t(title_key))).strong();
        let title = if is_miss(row.worst) {
            title.color(ui.visuals().warn_fg_color)
        } else {
            title
        };
        ui.label(title);
        ui.weak(i18n.t(source_key(row.source)));
        ui.label(count("indexes.queries.runs", row.executions, i18n));
        ui.label(count("indexes.queries.misses", row.misses, i18n));
        ui.label(count("indexes.queries.slowest", row.max_millis, i18n));
    });
    copyable(ui, &row.shape_json, "indexes.queries.copy_shape", i18n);
    if let Some(command) = &row.suggestion {
        ui.label(i18n.t("query.verdict.suggestion"));
        copyable(ui, command, "query.verdict.copy", i18n);
    }
}

/// `text` in monospace with a copy button named by `copy_key`.
fn copyable(ui: &mut egui::Ui, text: &str, copy_key: &str, i18n: &I18n) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(text).monospace());
        if ui.button(i18n.t(copy_key)).clicked() {
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
