//! The hint part of the options form: a combo of the collection's indexes (with its loading and
//! "could not list" states), the field for a name or key pattern, and Clear. A name that is not
//! among the listed indexes is a note, never a block: the server answers for it.
use super::aggregate_options_fields::{pick_combo, typed_field};
use super::options_form::{FormContext, FormLayout, Gathered, problem, width};
use crate::I18n;
use crate::theme::text_button;
use crate::viewmodel::aggregate_hint_list::{HintList, HintListUi};
use crate::viewmodel::aggregate_options::{OptionField, OptionsEdit, OptionsForm};
use eframe::egui;

pub(super) fn show_hint_group(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    hints: &HintListUi,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    cx.cell(ui, width::PICK, true, |ui| {
        show_index_list(ui, cx, &hints.list, &form.hint_text, g, i18n);
    });
    cx.cell(ui, width::FIELD, true, |ui| {
        let typed = typed_field(
            ui,
            cx.id_scope,
            &i18n.t("aggregate.options.hint.label"),
            &form.hint_text,
            &i18n.t("aggregate.options.hint.placeholder"),
            problem(form, OptionField::Hint, i18n),
        );
        g.typed(typed, OptionsEdit::HintText);
    });
    // The Clear button sits on the baseline of the field's row: in the wrapping layout it is a
    // cell of its own, in the columns layout it comes right after the field.
    cx.cell(ui, width::BUTTON, true, |ui| {
        if text_button(ui, i18n.t("aggregate.options.hint.clear")).clicked() {
            g.edits.push(OptionsEdit::HintCleared);
        }
    });
    let typed_name = form.hint_text.trim();
    let named = !typed_name.is_empty() && !typed_name.starts_with('{');
    if named
        && matches!(hints.list, HintList::Loaded(_))
        && !hints.names().iter().any(|n| n == typed_name)
    {
        match cx.layout {
            FormLayout::Columns | FormLayout::Wrapped => {
                ui.weak(i18n.t("aggregate.options.hint.not_listed"));
            }
        }
    }
}

/// The indexes combo, or what stands in its place while they are listed or when they could not
/// be.
fn show_index_list(
    ui: &mut egui::Ui,
    cx: &FormContext<'_>,
    list: &HintList,
    typed: &str,
    g: &mut Gathered,
    i18n: &I18n,
) {
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
            let picked = pick_combo(ui, cx.id_scope, "hint_existing", &label, current, &choices);
            g.pick(picked, |i| OptionsEdit::HintPicked(names[i].clone()));
        }
    }
}
