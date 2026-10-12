//! The detail pane: field-level change lines and the two documents as JSON.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{ChangeLine, DetailView, key_label};
use bson::Document;
use curlybrace_core::ejson::doc_to_view_json;
use eframe::egui;

const DOC_MAX_HEIGHT: f32 = 240.0;

/// Pretty JSON of both documents, formatted once per opened detail rather than every frame.
type JsonCache = (Option<String>, Option<String>);

fn cache_id() -> egui::Id {
    egui::Id::new("compare_detail_json")
}

/// Drops the cached JSON; called whenever no loaded detail is on screen.
pub fn forget_cache(ui: &egui::Ui) {
    ui.data_mut(|d| d.remove::<JsonCache>(cache_id()));
}

pub fn show_detail(ui: &mut egui::Ui, detail: &DetailView, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let key = key_label(&detail.key);
        ui.strong(i18n.t_with("compare.detail.title", &[("key", &key)]));
        if ui.button(i18n.t("compare.detail.close")).clicked() {
            action = Some(CompareAction::Back);
        }
    });
    if detail.loading {
        forget_cache(ui);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(i18n.t("compare.detail.loading"));
        });
        return action;
    }
    show_changes(ui, &detail.changes, i18n);
    show_documents(ui, detail, i18n);
    action
}

fn show_changes(ui: &mut egui::Ui, changes: &[ChangeLine], i18n: &I18n) {
    if changes.is_empty() {
        ui.label(i18n.t("compare.detail.no_changes"));
        return;
    }
    egui::Grid::new("compare_changes")
        .striped(true)
        .show(ui, |ui| {
            for key in [
                "compare.detail.change_column",
                "compare.detail.path_column",
                "compare.detail.source_value",
                "compare.detail.target_value",
            ] {
                ui.strong(i18n.t(key));
            }
            ui.end_row();
            for (index, line) in changes.iter().enumerate() {
                ui.push_id(index, |ui| show_change(ui, line, i18n));
                ui.end_row();
            }
        });
}

fn show_change(ui: &mut egui::Ui, line: &ChangeLine, i18n: &I18n) {
    ui.label(i18n.t(line.kind));
    ui.monospace(&line.path);
    for value in [&line.source_text, &line.target_text] {
        match value {
            Some(text) => ui.monospace(text),
            None => ui.weak(i18n.t("compare.detail.absent")),
        };
    }
}

fn show_documents(ui: &mut egui::Ui, detail: &DetailView, i18n: &I18n) {
    let id = cache_id();
    let cached = ui.data(|d| d.get_temp::<JsonCache>(id));
    let (source, target) = cached.unwrap_or_else(|| {
        let formatted = (pretty(&detail.source), pretty(&detail.target));
        ui.data_mut(|d| d.insert_temp(id, formatted.clone()));
        formatted
    });
    ui.columns(2, |columns| {
        document_box(
            &mut columns[0],
            "source",
            "compare.detail.source_document",
            &source,
            i18n,
        );
        document_box(
            &mut columns[1],
            "target",
            "compare.detail.target_document",
            &target,
            i18n,
        );
    });
}

fn pretty(doc: &Option<Document>) -> Option<String> {
    doc.as_ref().map(doc_to_view_json)
}

fn document_box(ui: &mut egui::Ui, id: &str, title_key: &str, json: &Option<String>, i18n: &I18n) {
    ui.push_id(id, |ui| {
        ui.strong(i18n.t(title_key));
        let Some(text) = json else {
            ui.weak(i18n.t("compare.detail.no_document"));
            return;
        };
        egui::ScrollArea::both()
            .id_salt("json")
            .max_height(DOC_MAX_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(text).monospace()).selectable(true));
            });
    });
}
