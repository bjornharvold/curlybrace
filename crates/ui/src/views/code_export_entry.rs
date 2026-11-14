//! The Copy as code buttons of the two entry points (the Aggregate tab's header and the query
//! bar): one button, off with its reason as the tooltip and the accessible description when there
//! is nothing valid to show code for. Nothing here needs a connection.
use super::aggregate::pipeline_error_text;
use crate::I18n;
use crate::theme::{gate, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::code_export::{CodeBlock, aggregate_stages};
use curlybrace_core::query::{QueryText, parse_query};
use eframe::egui;

/// Why the Aggregate tab has no code to offer, in words; `None` when it has.
pub fn aggregate_blocked(draft: &AggregateState, i18n: &I18n) -> Option<String> {
    match aggregate_stages(draft) {
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

/// [`show_copy_code_button`]'s response, for a row that measures its buttons.
pub fn show_copy_code_response(
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

#[cfg(test)]
#[path = "code_export_entry_tests.rs"]
mod tests;
