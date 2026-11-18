//! The "Include options" check box of the Copy as code dialog and the line saying which options
//! the code sets (and that the read preference was left out when the code writes).
use crate::I18n;
use crate::app::CodeAction;
use crate::viewmodel::code_export_options::CodeOptionsView;
use eframe::egui;

/// The check box; the action when it changed this frame, and its id (Enter on it toggles it, so
/// it never also copies).
pub(super) fn show_include_options(
    ui: &mut egui::Ui,
    view: &CodeOptionsView,
    i18n: &I18n,
) -> (Option<CodeAction>, egui::Id) {
    let mut include = view.include;
    let response = ui.checkbox(&mut include, i18n.t(view.include_key));
    let action = response
        .changed()
        .then_some(CodeAction::SetIncludeOptions(include));
    (action, response.id)
}

/// "The code sets: ..." with the options in words, then the left-out note when it applies.
pub(super) fn show_options_summary(ui: &mut egui::Ui, view: &CodeOptionsView, i18n: &I18n) {
    if !view.labels.is_empty() {
        let names: Vec<String> = view.labels.iter().map(|key| i18n.t(key)).collect();
        let joined = names.join(&i18n.t("codegen.options.separator"));
        let listed = i18n.t_with("codegen.options.listed", &[("options", &joined)]);
        ui.label(listed);
    }
    if view.read_preference_left_out {
        ui.label(i18n.t("codegen.options.read_preference_left_out"));
    }
}

#[cfg(test)]
#[path = "code_export_options_tests.rs"]
mod tests;
