//! The detail pane: field-level change lines and the two documents as JSON.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{ChangeLine, DetailView, canonical_json, key_label};
use bson::Document;
use curlybrace_core::compare::key::KeyTuple;
use eframe::egui;
use std::sync::Arc;

const DOC_MAX_HEIGHT: f32 = 240.0;

/// Pretty canonical JSON of both documents of one key, formatted once rather than every frame.
/// Tagged with the key it was built for, so a different key never shows these texts.
type JsonCache = (KeyTuple, Json, Json);
type Json = Option<Arc<str>>;

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
            for line in changes {
                show_change(ui, line, i18n);
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
    let cached = ui
        .data(|d| d.get_temp::<JsonCache>(id))
        .filter(|(key, _, _)| detail.is_for(key));
    let (_, source, target) = cached.unwrap_or_else(|| {
        let built = (
            detail.key.clone(),
            pretty(&detail.source),
            pretty(&detail.target),
        );
        ui.data_mut(|d| d.insert_temp(id, built.clone()));
        built
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

fn pretty(doc: &Option<Document>) -> Json {
    doc.as_ref().map(|d| Arc::from(canonical_json(d)))
}

fn document_box(ui: &mut egui::Ui, id: &str, title_key: &str, json: &Json, i18n: &I18n) {
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
                let rich = egui::RichText::new(&**text).monospace();
                ui.add(
                    egui::Label::new(rich)
                        .selectable(true)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Bson, doc};

    fn loaded(n: i32, source: Document) -> DetailView {
        DetailView::loading(KeyTuple(vec![Bson::Int32(n)])).loaded(Some(source), None, &[])
    }

    fn texts(ctx: &egui::Context, detail: &DetailView) -> Vec<String> {
        let i18n = I18n::english();
        let mut out = Vec::new();
        for _ in 0..2 {
            out.clear();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show_detail(ui, detail, &i18n);
                });
            });
            for clipped in &output.shapes {
                if let egui::Shape::Text(t) = &clipped.shape {
                    out.push(t.galley.text().to_string());
                }
            }
        }
        out
    }

    #[test]
    fn a_second_loaded_detail_never_shows_the_first_ones_documents() {
        let ctx = egui::Context::default();
        let a = loaded(1, doc! {"_id": 1, "mark": "first"});
        let b = loaded(2, doc! {"_id": 2, "mark": "second"});
        assert!(texts(&ctx, &a).iter().any(|t| t.contains("first")));
        // No loading frame between the two: only the key tells the details apart.
        let shown = texts(&ctx, &b);
        assert!(shown.iter().any(|t| t.contains("second")), "{shown:?}");
        assert!(!shown.iter().any(|t| t.contains("first")), "{shown:?}");
    }

    #[test]
    fn documents_are_shown_as_canonical_extended_json() {
        let ctx = egui::Context::default();
        let int = loaded(1, doc! {"n": 1_i32});
        let long = loaded(2, doc! {"n": 1_i64});
        assert!(texts(&ctx, &int).iter().any(|t| t.contains("$numberInt")));
        assert!(texts(&ctx, &long).iter().any(|t| t.contains("$numberLong")));
    }
}
