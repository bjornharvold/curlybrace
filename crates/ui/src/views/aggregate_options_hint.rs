//! The hint part of the Options section: a combo of the collection's indexes (with its loading
//! and "could not list" states), the field for a name or key pattern, and Clear. A name that is
//! not among the listed indexes is a note, never a block: the server answers for it.
use super::aggregate_options::Gathered;
use super::aggregate_options_fields::{pick_combo, typed_field};
use crate::I18n;
use crate::theme::text_button;
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_hint_list::HintList;
use crate::viewmodel::aggregate_options::{OptionField, OptionsEdit, problem_message};
use eframe::egui;

pub(super) fn show_hint_group(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    g: &mut Gathered,
    i18n: &I18n,
) {
    let form = &draft.options_ui.form;
    let list = &draft.options_ui.hint;
    show_index_list(ui, &list.list, &form.hint_text, g, i18n);
    let problem = form
        .problem(OptionField::Hint)
        .map(|e| problem_message(i18n, e));
    let typed = typed_field(
        ui,
        &i18n.t("aggregate.options.hint.label"),
        &form.hint_text,
        &i18n.t("aggregate.options.hint.placeholder"),
        problem,
    );
    g.typed(typed, OptionsEdit::HintText);
    if text_button(ui, i18n.t("aggregate.options.hint.clear")).clicked() {
        g.edits.push(OptionsEdit::HintCleared);
    }
    let typed_name = form.hint_text.trim();
    let named = !typed_name.is_empty() && !typed_name.starts_with('{');
    if named
        && matches!(list.list, HintList::Loaded(_))
        && !list.names().iter().any(|n| n == typed_name)
    {
        ui.weak(i18n.t("aggregate.options.hint.not_listed"));
    }
}

/// The indexes combo, or what stands in its place while they are listed or when they could not
/// be.
fn show_index_list(ui: &mut egui::Ui, list: &HintList, typed: &str, g: &mut Gathered, i18n: &I18n) {
    match list {
        HintList::NotAsked | HintList::Loading { .. } => {
            ui.label(i18n.t("aggregate.options.hint.loading"));
        }
        HintList::Failed => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                i18n.t("aggregate.options.hint.list_failed"),
            );
        }
        HintList::Loaded(names) => {
            let choices: Vec<(usize, String)> = names.iter().cloned().enumerate().collect();
            let current = names
                .iter()
                .position(|n| n == typed.trim())
                .unwrap_or(usize::MAX);
            let label = i18n.t("aggregate.options.hint.existing");
            let picked = pick_combo(ui, "hint_existing", &label, current, &choices);
            g.pick(picked, |i| OptionsEdit::HintPicked(names[i].clone()));
        }
    }
}
