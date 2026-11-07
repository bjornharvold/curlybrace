//! The index verdict as one fixed-height row above the documents: icon and title as one text,
//! plus a "Why" button that opens the reasons, the suggested index (with a copy button) and the
//! estimate note in a popup under the row. The row is reserved even when there is no verdict
//! and the popup floats, so the documents never move.
use crate::I18n;
use crate::viewmodel::verdict::{BadgeModel, Tone, reason_text_key};
use eframe::egui;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Debug, Clone, PartialEq)]
pub enum VerdictAction {
    CopySuggestion(String),
}

/// Draws the row for `model` (an empty row of the same height for `None`).
pub fn show_verdict_row(
    ui: &mut egui::Ui,
    model: Option<&BadgeModel>,
    i18n: &I18n,
) -> Option<VerdictAction> {
    let height = ui.spacing().interact_size.y;
    let mut action = None;
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), layout, |ui| {
        ui.set_min_height(height);
        action = model.and_then(|model| show_row(ui, model, i18n));
    });
    action
}

fn show_row(ui: &mut egui::Ui, model: &BadgeModel, i18n: &I18n) -> Option<VerdictAction> {
    let title = format!("{} {}", model.icon, i18n.t(model.title_key));
    ui.label(
        egui::RichText::new(title)
            .strong()
            .color(tone_color(ui, model.tone)),
    );
    if !has_details(model) {
        return None;
    }
    let toggle = ui.button(i18n.t("query.verdict.why"));
    // Per verdict, so one verdict's open state does not carry over to an unrelated one.
    let open_id = ui.id().with(("verdict_why", salt(model)));
    let was_open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let mut open = was_open != toggle.clicked();
    let mut action = None;
    if open {
        let popup = egui::Area::new(open_id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(toggle.rect.left_bottom())
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| show_details(ui, model, i18n))
                    .inner
            });
        action = popup.inner;
        if popup.response.clicked_elsewhere() && !toggle.clicked() {
            open = false;
        }
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
    action
}

fn has_details(model: &BadgeModel) -> bool {
    !model.reasons.is_empty() || model.plan_only || model.suggestion.is_some()
}

fn salt(model: &BadgeModel) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{model:?}").hash(&mut hasher);
    hasher.finish()
}

fn tone_color(ui: &egui::Ui, tone: Tone) -> egui::Color32 {
    match tone {
        Tone::Warn => ui.visuals().warn_fg_color,
        _ => ui.visuals().text_color(),
    }
}

fn show_details(ui: &mut egui::Ui, model: &BadgeModel, i18n: &I18n) -> Option<VerdictAction> {
    ui.set_max_width(480.0);
    for reason in &model.reasons {
        ui.label(reason_line(reason.key, &reason.params, i18n));
    }
    if model.plan_only {
        ui.weak(i18n.t("query.verdict.plan_only"));
    }
    model
        .suggestion
        .as_deref()
        .and_then(|command| show_suggestion(ui, command, i18n))
}

/// A reason's text; one core emits that the badge has no string for still says something
/// visible (with its raw key) instead of vanishing.
fn reason_line(key: &str, params: &[(&'static str, String)], i18n: &I18n) -> String {
    match reason_text_key(key) {
        Some(full) => {
            let params: Vec<(&str, &str)> = params.iter().map(|(n, v)| (*n, v.as_str())).collect();
            i18n.t_with(full, &params)
        }
        None => i18n.t_with("query.verdict.reason_unknown", &[("key", key)]),
    }
}

fn show_suggestion(ui: &mut egui::Ui, command: &str, i18n: &I18n) -> Option<VerdictAction> {
    ui.label(i18n.t("query.verdict.suggestion"));
    ui.label(egui::RichText::new(command).monospace());
    ui.button(i18n.t("query.verdict.copy"))
        .clicked()
        .then(|| VerdictAction::CopySuggestion(command.to_string()))
}
