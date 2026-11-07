//! Draws the index verdict badge above the documents: icon and title as one text, the reasons
//! in a "why" section, the suggested index with a copy button, and a note when it is an estimate.
use crate::I18n;
use crate::viewmodel::verdict::{BadgeModel, Tone, reason_text_key};
use eframe::egui;

#[derive(Debug, Clone, PartialEq)]
pub enum VerdictAction {
    CopySuggestion(String),
}

pub fn show_verdict_badge(
    ui: &mut egui::Ui,
    model: &BadgeModel,
    i18n: &I18n,
) -> Option<VerdictAction> {
    let mut action = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let title = format!("{} {}", model.icon, i18n.t(model.title_key));
        ui.label(
            egui::RichText::new(title)
                .strong()
                .color(tone_color(ui, model.tone)),
        );
        show_reasons(ui, model, i18n);
        if model.plan_only {
            ui.weak(i18n.t("query.verdict.plan_only"));
        }
        action = model
            .suggestion
            .as_deref()
            .and_then(|command| show_suggestion(ui, command, i18n));
    });
    action
}

fn tone_color(ui: &egui::Ui, tone: Tone) -> egui::Color32 {
    let visuals = ui.visuals();
    match tone {
        Tone::Warn => visuals.warn_fg_color,
        Tone::Unavailable | Tone::Neutral | Tone::Checking => visuals.text_color(),
        Tone::Good => visuals.text_color(),
    }
}

fn show_reasons(ui: &mut egui::Ui, model: &BadgeModel, i18n: &I18n) {
    if model.reasons.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(i18n.t("query.verdict.why"))
        .id_salt("verdict_why")
        .default_open(model.tone == Tone::Warn)
        .show(ui, |ui| {
            for reason in &model.reasons {
                let Some(key) = reason_text_key(reason.key) else {
                    continue;
                };
                let params: Vec<(&str, &str)> = reason
                    .params
                    .iter()
                    .map(|(n, v)| (*n, v.as_str()))
                    .collect();
                ui.label(i18n.t_with(key, &params));
            }
        });
}

fn show_suggestion(ui: &mut egui::Ui, command: &str, i18n: &I18n) -> Option<VerdictAction> {
    ui.label(i18n.t("query.verdict.suggestion"));
    ui.label(egui::RichText::new(command).monospace());
    ui.button(i18n.t("query.verdict.copy"))
        .clicked()
        .then(|| VerdictAction::CopySuggestion(command.to_string()))
}
