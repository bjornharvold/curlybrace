//! The Options section of the Aggregate tab: the header toggle with its "Options (N)" indicator
//! and the section itself (collation on the left, the rest on the right above 900 pt, one column
//! below). Draws what it is given and returns what the person did; typed text is applied by the
//! caller (Enter, focus loss or a pause), the picks apply at once in the reducer.
use super::aggregate_options_fields::{Typed, pick_combo, tri_combo, typed_field};
use super::aggregate_options_hint::show_hint_group;
use crate::I18n;
use crate::app::{AggregateAction, OptionsAction};
use crate::theme::shape::{self, radius};
use crate::theme::{current_ui, filled_button, text_button, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_options::{
    COMMON_LOCALES, OptionField, OptionsEdit, OptionsForm, problem_message,
};
use crate::viewmodel::aggregate_options_view::read_preference_key;
use curlybrace_core::pipeline::{
    AGGREGATE_RUN_MAX_TIME_MS, Alternate, CaseFirst, MaxTimeEffect, Operation, PREVIEW_MAX_TIME_MS,
    ReadPreferenceMode, Strength,
};
use eframe::egui::{self, WidgetInfo, WidgetType};

/// The width above which the section lays out in two columns.
pub const TWO_COLUMNS_FROM: f32 = 900.0;

/// What the controls did this frame, before it becomes actions.
#[derive(Default)]
pub(super) struct Gathered {
    pub edits: Vec<OptionsEdit>,
    /// Typed text was committed (or a pick of typed text is to apply at once).
    pub commit: bool,
    pub reset: bool,
}

impl Gathered {
    /// A text field's change and its commit.
    pub(super) fn typed(&mut self, typed: Typed, make: impl FnOnce(String) -> OptionsEdit) {
        self.commit |= typed.commit;
        self.edits.extend(typed.text.map(make));
    }

    /// A pick.
    pub(super) fn pick<T>(&mut self, picked: Option<T>, make: impl FnOnce(T) -> OptionsEdit) {
        self.edits.extend(picked.map(make));
    }
}

/// "Options" or "Options (N)" with the accent; named with its expanded state.
pub(super) fn show_options_toggle(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    i18n: &I18n,
) -> Option<AggregateAction> {
    let set = draft.options.set_count();
    let open = draft.options_ui.open;
    let label = match set {
        0 => i18n.t("aggregate.options.toggle"),
        n => i18n.t_with("aggregate.options.toggle_count", &[("n", &n.to_string())]),
    };
    let response = match set {
        0 => tonal_button(ui, label.clone()),
        _ => filled_button(ui, label.clone()),
    }
    .on_hover_text(i18n.t("aggregate.options.toggle_hint"));
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, open, label.clone()));
    response
        .clicked()
        .then_some(AggregateAction::Options(OptionsAction::Toggle))
}

/// The open section; nothing while it is closed.
pub(super) fn show_options_section(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    if !draft.options_ui.open {
        return Vec::new();
    }
    let mut gathered = Gathered::default();
    section_frame(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match ui.available_width() > TWO_COLUMNS_FROM {
            true => {
                ui.columns(2, |columns| {
                    show_collation(&mut columns[0], &draft.options_ui.form, &mut gathered, i18n);
                    show_others(&mut columns[1], draft, &mut gathered, i18n);
                });
            }
            false => {
                show_collation(ui, &draft.options_ui.form, &mut gathered, i18n);
                show_others(ui, draft, &mut gathered, i18n);
            }
        }
    });
    let wanted = draft
        .options_ui
        .hint
        .wants_list()
        .then_some(AggregateAction::Options(OptionsAction::HintListWanted));
    let edits = gathered
        .edits
        .into_iter()
        .map(|e| AggregateAction::Options(OptionsAction::Edit(e)));
    let apply = gathered
        .commit
        .then_some(AggregateAction::Options(OptionsAction::Apply));
    let reset = gathered
        .reset
        .then_some(AggregateAction::Options(OptionsAction::Reset));
    wanted
        .into_iter()
        .chain(edits)
        .chain(apply)
        .chain(reset)
        .collect()
}

fn section_frame(ui: &egui::Ui) -> egui::Frame {
    let s = current_ui(ui);
    egui::Frame::new()
        .fill(s.surface_container_low)
        .stroke(egui::Stroke::new(1.0, s.outline_variant))
        .corner_radius(radius(shape::MEDIUM))
        .inner_margin(egui::Margin::same(shape::SPACE_2 as i8))
        .outer_margin(egui::Margin::symmetric(0, shape::SPACE_1 as i8))
}

fn problem(form: &OptionsForm, field: OptionField, i18n: &I18n) -> Option<String> {
    form.problem(field).map(|e| problem_message(i18n, e))
}

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

/// The collation checkbox, and under it (while it is on) the locale and the collation flags.
fn show_collation(ui: &mut egui::Ui, form: &OptionsForm, g: &mut Gathered, i18n: &I18n) {
    let label = i18n.t("aggregate.options.collation.enabled");
    let mut on = form.collation_on;
    let response = ui.checkbox(&mut on, label.clone());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, label.clone()));
    if response.changed() {
        g.edits.push(OptionsEdit::CollationOn(on));
    }
    ui.add_enabled_ui(form.collation_on, |ui| {
        let text = i18n.t("aggregate.options.collation.locale");
        let typed = typed_field(
            ui,
            &text,
            &form.locale,
            &i18n.t("aggregate.options.collation.locale_hint"),
            problem(form, OptionField::Locale, i18n),
        );
        g.typed(typed, OptionsEdit::Locale);
        let current = COMMON_LOCALES
            .iter()
            .copied()
            .find(|l| *l == form.locale)
            .unwrap_or("");
        let locales: Vec<(&str, String)> =
            COMMON_LOCALES.iter().map(|l| (*l, l.to_string())).collect();
        let picked = pick_combo(
            ui,
            "locale",
            &i18n.t("aggregate.options.collation.locale_pick"),
            current,
            &locales,
        );
        g.commit |= picked.is_some();
        g.pick(picked, |l| OptionsEdit::Locale(l.to_string()));
        show_collation_flags(ui, form, g, i18n);
    });
}

fn show_collation_flags(ui: &mut egui::Ui, form: &OptionsForm, g: &mut Gathered, i18n: &I18n) {
    let all_strengths = [
        Strength::Primary,
        Strength::Secondary,
        Strength::Tertiary,
        Strength::Quaternary,
        Strength::Identical,
    ];
    let label = i18n.t("aggregate.options.collation.strength");
    let strengths = choices(&all_strengths, strength_key, i18n);
    let picked = pick_combo(ui, "strength", &label, form.strength, &strengths);
    g.pick(picked, OptionsEdit::Strength);
    let label = i18n.t("aggregate.options.collation.case_level");
    let picked = tri_combo(ui, "case_level", &label, form.case_level, i18n);
    g.pick(picked, OptionsEdit::CaseLevel);
    let label = i18n.t("aggregate.options.collation.case_first");
    let firsts = choices(
        &[CaseFirst::Upper, CaseFirst::Lower, CaseFirst::Off],
        case_first_key,
        i18n,
    );
    let picked = pick_combo(ui, "case_first", &label, form.case_first, &firsts);
    g.pick(picked, OptionsEdit::CaseFirst);
    let label = i18n.t("aggregate.options.collation.numeric_ordering");
    let picked = tri_combo(ui, "numeric_ordering", &label, form.numeric_ordering, i18n);
    g.pick(picked, OptionsEdit::NumericOrdering);
    let label = i18n.t("aggregate.options.collation.alternate");
    let alternates = choices(
        &[Alternate::NonIgnorable, Alternate::Shifted],
        alternate_key,
        i18n,
    );
    let picked = pick_combo(ui, "alternate", &label, form.alternate, &alternates);
    g.pick(picked, OptionsEdit::Alternate);
}

/// The hint, disk use, time limit, comment and read preference, then Reset.
fn show_others(ui: &mut egui::Ui, draft: &AggregateState, g: &mut Gathered, i18n: &I18n) {
    let form = &draft.options_ui.form;
    show_hint_group(ui, draft, g, i18n);
    let label = i18n.t("aggregate.options.allow_disk_use.label");
    let picked = tri_combo(ui, "allow_disk_use", &label, form.allow_disk_use, i18n);
    g.pick(picked, OptionsEdit::AllowDiskUse);
    ui.weak(i18n.t("aggregate.options.allow_disk_use.preview_note"));
    show_max_time(ui, draft, g, i18n);
    let typed = typed_field(
        ui,
        &i18n.t("aggregate.options.comment.label"),
        &form.comment,
        &i18n.t("aggregate.options.comment.placeholder"),
        problem(form, OptionField::Comment, i18n),
    );
    g.typed(typed, OptionsEdit::Comment);
    ui.weak(i18n.t("aggregate.options.comment_sent_note"));
    let modes = [
        None,
        Some(ReadPreferenceMode::Primary),
        Some(ReadPreferenceMode::PrimaryPreferred),
        Some(ReadPreferenceMode::Secondary),
        Some(ReadPreferenceMode::SecondaryPreferred),
        Some(ReadPreferenceMode::Nearest),
    ];
    let modes: Vec<(Option<ReadPreferenceMode>, String)> = modes
        .into_iter()
        .map(|m| (m, i18n.t(read_preference_key(m))))
        .collect();
    let label = i18n.t("aggregate.options.read_preference.label");
    let picked = pick_combo(ui, "read_preference", &label, form.read_preference, &modes);
    g.pick(picked, OptionsEdit::ReadPreference);
    ui.add_space(shape::SPACE_1);
    g.reset |= text_button(ui, i18n.t("aggregate.options.reset")).clicked();
}

/// The time limit field, with the defaults as its placeholder and a note when the limit is
/// above what previews allow.
fn show_max_time(ui: &mut egui::Ui, draft: &AggregateState, g: &mut Gathered, i18n: &I18n) {
    let form = &draft.options_ui.form;
    let preview = (PREVIEW_MAX_TIME_MS / 1000).to_string();
    let run = (AGGREGATE_RUN_MAX_TIME_MS / 1000).to_string();
    let placeholder = i18n.t_with(
        "aggregate.options.max_time.placeholder",
        &[("preview", &preview), ("run", &run)],
    );
    let typed = typed_field(
        ui,
        &i18n.t("aggregate.options.max_time.label"),
        &form.max_time,
        &placeholder,
        problem(form, OptionField::MaxTime, i18n),
    );
    g.typed(typed, OptionsEdit::MaxTime);
    if draft.options.max_time_effect(Operation::Preview) == MaxTimeEffect::Capped {
        ui.weak(i18n.t_with("aggregate.options.max_time.clamped", &[("limit", &preview)]));
    }
}

#[cfg(test)]
#[path = "aggregate_options_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "aggregate_options_status_tests.rs"]
mod status_tests;
