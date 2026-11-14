//! The Copy as code buttons of the two entry points (the Aggregate tab's header and the query
//! bar): one button, off with its reason as the tooltip and the accessible description when there
//! is nothing valid to show code for. Nothing here needs a connection.
use super::aggregate::pipeline_error_text;
use crate::I18n;
use crate::theme::{compact_text_button, gate, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::code_export::{CodeBlock, stages_exported};
use curlybrace_core::pipeline::Stage;
use curlybrace_core::query::{QueryText, parse_query};
use eframe::egui::{self, WidgetInfo, WidgetType};

/// Why the Aggregate tab has no code to offer, in words; `None` when it has.
/// `stages` are the draft's own, or with the text still being typed applied.
pub fn aggregate_blocked(draft: &AggregateState, stages: &[Stage], i18n: &I18n) -> Option<String> {
    match stages_exported(draft, stages) {
        Ok(_) => None,
        Err(CodeBlock::Pipeline(error)) => Some(pipeline_error_text(&error, i18n)),
        Err(CodeBlock::TextNotApplied) => Some(i18n.t("codegen.disabled.text_not_applied")),
    }
}

/// Why the query bar has no code to offer (its text does not parse), in words.
pub fn find_blocked(text: &QueryText, i18n: &I18n) -> Option<String> {
    parse_query(text).err().map(|error| i18n.error(&error))
}

/// The Copy as code button; true when it was clicked while on. `blocked` is the reason it is
/// off.
pub fn show_copy_code_button(ui: &mut egui::Ui, blocked: Option<&str>, i18n: &I18n) -> bool {
    show_copy_code_response(ui, blocked, i18n).clicked() && blocked.is_none()
}

fn show_copy_code_response(
    ui: &mut egui::Ui,
    blocked: Option<&str>,
    i18n: &I18n,
) -> egui::Response {
    let label = i18n.t("codegen.button");
    match blocked {
        None => tonal_button(ui, label).on_hover_text(i18n.t("codegen.button_hint")),
        Some(reason) => {
            let off = gate(ui, false, |ui| tonal_button(ui, label)).on_hover_text(reason);
            let why = reason.to_string();
            ui.ctx()
                .accesskit_node_builder(off.id, |node| node.set_description(why));
            off
        }
    }
}

/// The query bar's Copy as code button: the icon-only `</>` glyph, so it does not make the row
/// wrap. "Copy as code" is its accessible name and its tooltip; off, the tooltip and the
/// accessible description also give the reason. The response of the button.
pub fn show_copy_code_icon(
    ui: &mut egui::Ui,
    blocked: Option<&str>,
    i18n: &I18n,
) -> egui::Response {
    let name = i18n.t("codegen.button");
    let glyph = i18n.t("codegen.button_icon");
    let enabled = blocked.is_none();
    let response = match blocked {
        None => compact_text_button(ui, glyph).on_hover_text(name.clone()),
        Some(reason) => gate(ui, false, |ui| compact_text_button(ui, glyph)).on_hover_ui(|ui| {
            ui.strong(name.clone());
            ui.label(reason);
        }),
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name.clone()));
    let description = blocked.map_or_else(|| i18n.t("codegen.button_hint"), str::to_string);
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(description));
    response
}

#[cfg(test)]
#[path = "code_export_entry_tests.rs"]
mod tests;
