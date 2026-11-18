//! The collation part of the options form: the checkbox, the locale (typed, or picked from the
//! common ones) and the collation flags. In the columns layout the flags are disabled until the
//! collation is ticked; in the wrapping layout they are not drawn until then.
use super::aggregate_options_fields::{pick_combo, tri_combo, typed_field};
use super::options_form::{FlagsWhenOff, FormContext, FormLayout, Gathered, problem, width};
use crate::I18n;
use crate::viewmodel::aggregate_options::{COMMON_LOCALES, OptionField, OptionsEdit, OptionsForm};
use curlybrace_core::pipeline::{Alternate, CaseFirst, Strength};
use eframe::egui::{self, WidgetInfo, WidgetType};

fn strength_key(strength: Option<Strength>) -> &'static str {
    match strength {
        None => "aggregate.options.collation.server_default",
        Some(Strength::Primary) => "aggregate.options.collation.strength_value.1",
        Some(Strength::Secondary) => "aggregate.options.collation.strength_value.2",
        Some(Strength::Tertiary) => "aggregate.options.collation.strength_value.3",
        Some(Strength::Quaternary) => "aggregate.options.collation.strength_value.4",
        Some(Strength::Identical) => "aggregate.options.collation.strength_value.5",
    }
}

fn case_first_key(value: Option<CaseFirst>) -> &'static str {
    match value {
        None => "aggregate.options.collation.server_default",
        Some(CaseFirst::Upper) => "aggregate.options.collation.case_first_value.upper",
        Some(CaseFirst::Lower) => "aggregate.options.collation.case_first_value.lower",
        Some(CaseFirst::Off) => "aggregate.options.collation.case_first_value.off",
    }
}

fn alternate_key(value: Option<Alternate>) -> &'static str {
    match value {
        None => "aggregate.options.collation.server_default",
        Some(Alternate::NonIgnorable) => {
            "aggregate.options.collation.alternate_value.non_ignorable"
        }
        Some(Alternate::Shifted) => "aggregate.options.collation.alternate_value.shifted",
    }
}

/// The choices of a combo over `Option<T>`: none (the server default) and each value.
fn choices<T: Copy>(
    values: &[T],
    key: fn(Option<T>) -> &'static str,
    i18n: &I18n,
) -> Vec<(Option<T>, String)> {
    std::iter::once(None)
        .chain(values.iter().copied().map(Some))
        .map(|v| (v, i18n.t(key(v))))
        .collect()
}

/// The collation checkbox, and after it (while it is on) the locale and the collation flags.
pub(super) fn show_collation(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let label = i18n.t("aggregate.options.collation.enabled");
    let mut on = form.collation_on;
    let response = ui.checkbox(&mut on, label.clone());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, label.clone()));
    if response.changed() {
        g.edits.push(OptionsEdit::CollationOn(on));
    }
    match cx.layout {
        FormLayout::Columns => {
            ui.add_enabled_ui(form.collation_on, |ui| {
                show_locale_and_flags(ui, form, g, cx, i18n)
            });
        }
        FormLayout::Wrapped => show_locale_and_flags(ui, form, g, cx, i18n),
    }
}

fn show_locale_and_flags(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let on = form.collation_on;
    cx.cell(ui, width::FIELD, on, |ui| {
        show_locale(ui, form, g, cx, i18n)
    });
    cx.cell(ui, width::PICK, on, |ui| {
        show_locale_pick(ui, form, g, cx, i18n)
    });
    cx.cell(ui, width::PICK, on, |ui| {
        let all = [
            Strength::Primary,
            Strength::Secondary,
            Strength::Tertiary,
            Strength::Quaternary,
            Strength::Identical,
        ];
        let label = i18n.t("aggregate.options.collation.strength");
        let strengths = choices(&all, strength_key, i18n);
        let picked = pick_combo(
            ui,
            cx.id_scope,
            "strength",
            &label,
            form.strength,
            &strengths,
        );
        g.pick(picked, OptionsEdit::Strength);
    });
    let flags_hidden = cx.flags_when_off == FlagsWhenOff::Hidden && !on;
    if !flags_hidden {
        show_flags(ui, form, g, cx, i18n);
    }
}

fn show_locale(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let text = i18n.t("aggregate.options.collation.locale");
    let typed = typed_field(
        ui,
        cx.id_scope,
        &text,
        &form.locale,
        &i18n.t("aggregate.options.collation.locale_hint"),
        problem(form, OptionField::Locale, i18n),
    );
    let left = typed.commit;
    g.typed(typed, OptionsEdit::Locale);
    // Leaving the empty field is an edit: from here on an empty locale is a problem.
    if left && form.locale.is_empty() {
        g.edits.push(OptionsEdit::Locale(String::new()));
    }
}

fn show_locale_pick(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let current = COMMON_LOCALES
        .iter()
        .copied()
        .find(|l| *l == form.locale)
        .unwrap_or("");
    let locales: Vec<(&str, String)> = COMMON_LOCALES.iter().map(|l| (*l, l.to_string())).collect();
    let picked = pick_combo(
        ui,
        cx.id_scope,
        "locale",
        &i18n.t("aggregate.options.collation.locale_pick"),
        current,
        &locales,
    );
    g.commit |= picked.is_some();
    g.pick(picked, |l| OptionsEdit::Locale(l.to_string()));
}

fn show_flags(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let on = form.collation_on;
    let scope = cx.id_scope;
    cx.cell(ui, width::FLAG, on, |ui| {
        let label = i18n.t("aggregate.options.collation.case_level");
        let picked = tri_combo(ui, scope, "case_level", &label, form.case_level, i18n);
        g.pick(picked, OptionsEdit::CaseLevel);
    });
    cx.cell(ui, width::FLAG, on, |ui| {
        let label = i18n.t("aggregate.options.collation.case_first");
        let firsts = choices(
            &[CaseFirst::Upper, CaseFirst::Lower, CaseFirst::Off],
            case_first_key,
            i18n,
        );
        let picked = pick_combo(ui, scope, "case_first", &label, form.case_first, &firsts);
        g.pick(picked, OptionsEdit::CaseFirst);
    });
    cx.cell(ui, width::FLAG, on, |ui| {
        let label = i18n.t("aggregate.options.collation.numeric_ordering");
        let picked = tri_combo(
            ui,
            scope,
            "numeric_ordering",
            &label,
            form.numeric_ordering,
            i18n,
        );
        g.pick(picked, OptionsEdit::NumericOrdering);
    });
    cx.cell(ui, width::PICK, on, |ui| {
        let label = i18n.t("aggregate.options.collation.alternate");
        let alternates = choices(
            &[Alternate::NonIgnorable, Alternate::Shifted],
            alternate_key,
            i18n,
        );
        let picked = pick_combo(ui, scope, "alternate", &label, form.alternate, &alternates);
        g.pick(picked, OptionsEdit::Alternate);
    });
}
