//! The options form shared by the Aggregate tab and the query bar: the collation, the hint, the
//! disk use, the time limit, the comment and the read preference. It draws a form it is given and
//! returns what the person did; the owner decides when typed text is applied and what a pick
//! means. Two layouts: columns (the Aggregate tab) and wrapping rows (the query bar). The id scope
//! keeps the widgets of two forms apart.
use super::aggregate_options_fields::{Typed, pick_combo, tri_combo, typed_field};
use super::aggregate_options_hint::show_hint_group;
use super::options_form_collation::show_collation;
use crate::I18n;
use crate::theme::shape;
use crate::theme::text_button;
use crate::viewmodel::aggregate_hint_list::HintListUi;
use crate::viewmodel::aggregate_options::{OptionField, OptionsEdit, OptionsForm, problem_message};
use crate::viewmodel::aggregate_options_view::read_preference_key;
use curlybrace_core::pipeline::ReadPreferenceMode;
use eframe::egui;

/// The width above which the columns layout has two columns.
pub const TWO_COLUMNS_FROM: f32 = 900.0;

/// What the controls did this frame, before it becomes actions.
#[derive(Default)]
pub(super) struct Gathered {
    pub edits: Vec<OptionsEdit>,
    /// Typed text was committed (or a pick of typed text is to apply at once).
    pub commit: bool,
    pub reset: bool,
    /// Enter was pressed in a text field (a commit that is not just a focus loss).
    pub enter: bool,
}

impl Gathered {
    /// A text field's change and its commit.
    pub(super) fn typed(&mut self, typed: Typed, make: impl FnOnce(String) -> OptionsEdit) {
        self.commit |= typed.commit;
        self.enter |= typed.enter;
        self.edits.extend(typed.text.map(make));
    }

    /// A pick.
    pub(super) fn pick<T>(&mut self, picked: Option<T>, make: impl FnOnce(T) -> OptionsEdit) {
        self.edits.extend(picked.map(make));
    }
}

/// How the form is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FormLayout {
    /// Collation on the left, the rest on the right above [`TWO_COLUMNS_FROM`], one column below.
    Columns,
    /// Wrapping rows of labelled controls, in reading order (the query bar).
    Wrapped,
}

/// What the collation's flags do while the collation is not ticked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FlagsWhenOff {
    Disabled,
    Hidden,
}

/// What differs between the two owners of the form.
pub(super) struct FormContext<'a> {
    /// Salts every widget id, so two forms in one frame never share a widget.
    pub id_scope: &'a str,
    pub layout: FormLayout,
    /// The note under the disk use choice.
    pub disk_use_note: &'static str,
    pub comment_placeholder: &'static str,
    pub max_time_placeholder: (&'static str, Vec<(&'static str, String)>),
    /// The note under the time limit when it is above what the operation allows.
    pub max_time_clamped: Option<(&'static str, Vec<(&'static str, String)>)>,
    pub flags_when_off: FlagsWhenOff,
}

/// The widths of the controls in the wrapping layout: a flag combo, a picker, a text field, the
/// comment, a button. A control has its width before it is drawn, so the row wraps whole cells.
pub(super) mod width {
    pub const FLAG: f32 = 130.0;
    pub const PICK: f32 = 170.0;
    pub const FIELD: f32 = 190.0;
    pub const COMMENT: f32 = 280.0;
    pub const BUTTON: f32 = 90.0;
}

impl FormContext<'_> {
    /// A control with its label: in the wrapping layout a cell `width` wide (disabled while
    /// `enabled` is off), in the columns layout drawn as it comes.
    pub(super) fn cell(
        &self,
        ui: &mut egui::Ui,
        width: f32,
        enabled: bool,
        add: impl FnOnce(&mut egui::Ui),
    ) {
        match self.layout {
            FormLayout::Columns => add(ui),
            FormLayout::Wrapped => {
                // Top down inside the cell, so a label sits above its control as in the columns.
                let layout = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(egui::vec2(width, 0.0), layout, |ui| {
                    ui.add_enabled_ui(enabled, add);
                });
            }
        }
    }
}

/// The words of the problem of `field`, if it has one.
pub(super) fn problem(form: &OptionsForm, field: OptionField, i18n: &I18n) -> Option<String> {
    form.problem(field).map(|e| problem_message(i18n, e))
}

/// The form, in the layout of `cx`; what the person did this frame.
pub(super) fn show_options_form(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    hints: &HintListUi,
    cx: &FormContext<'_>,
    i18n: &I18n,
) -> Gathered {
    let mut g = Gathered::default();
    match cx.layout {
        FormLayout::Columns => show_columns(ui, form, hints, cx, &mut g, i18n),
        FormLayout::Wrapped => show_wrapped(ui, form, hints, cx, &mut g, i18n),
    }
    g
}

fn show_columns(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    hints: &HintListUi,
    cx: &FormContext<'_>,
    g: &mut Gathered,
    i18n: &I18n,
) {
    // The window's width, not the pane's: the sidebar takes a share of it, and a pane of 700 pt
    // in a 1000 pt window still has room for two columns.
    match ui.ctx().screen_rect().width() > TWO_COLUMNS_FROM {
        true => {
            ui.columns(2, |columns| {
                show_collation(&mut columns[0], form, g, cx, i18n);
                show_others(&mut columns[1], form, hints, g, cx, i18n);
            });
        }
        false => {
            show_collation(ui, form, g, cx, i18n);
            show_others(ui, form, hints, g, cx, i18n);
        }
    }
}

/// A row that wraps its cells and lines them up by their tops, so a row is as tall as its
/// tallest cell and no taller.
fn wrapped_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let layout = egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true);
    ui.with_layout(layout, add);
}

/// Three wrapping rows: the collation; the index list, hint, time limit, read preference and
/// disk use; the comment, the notes and Reset.
fn show_wrapped(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    hints: &HintListUi,
    cx: &FormContext<'_>,
    g: &mut Gathered,
    i18n: &I18n,
) {
    wrapped_row(ui, |ui| show_collation(ui, form, g, cx, i18n));
    wrapped_row(ui, |ui| {
        show_hint_group(ui, form, hints, g, cx, i18n);
        cx.cell(ui, width::FIELD, true, |ui| {
            show_max_time(ui, form, g, cx, i18n)
        });
        cx.cell(ui, width::PICK, true, |ui| {
            show_read_preference(ui, form, g, cx, i18n)
        });
        cx.cell(ui, width::FLAG, true, |ui| {
            show_disk_use(ui, form, g, cx, i18n)
        });
    });
    wrapped_row(ui, |ui| {
        cx.cell(ui, width::COMMENT, true, |ui| {
            show_comment(ui, form, g, cx, i18n)
        });
        ui.add_space(shape::SPACE_1);
        g.reset |= text_button(ui, i18n.t("aggregate.options.reset")).clicked();
    });
    // Whole lines under the rows: a note wrapped into what a row has left would be a tall sliver.
    show_notes(ui, form, cx, i18n);
}

/// The hint, disk use, time limit, comment and read preference, then Reset.
fn show_others(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    hints: &HintListUi,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    show_hint_group(ui, form, hints, g, cx, i18n);
    show_disk_use(ui, form, g, cx, i18n);
    ui.weak(i18n.t(cx.disk_use_note));
    show_max_time(ui, form, g, cx, i18n);
    if let Some((key, params)) = &cx.max_time_clamped {
        ui.weak(note_text(i18n, key, params));
    }
    show_comment(ui, form, g, cx, i18n);
    ui.weak(i18n.t("aggregate.options.comment_sent_note"));
    show_read_preference(ui, form, g, cx, i18n);
    ui.add_space(shape::SPACE_1);
    g.reset |= text_button(ui, i18n.t("aggregate.options.reset")).clicked();
}

/// What the wrapping layout says after its rows: the disk use, time limit and comment notes.
fn show_notes(ui: &mut egui::Ui, _form: &OptionsForm, cx: &FormContext<'_>, i18n: &I18n) {
    ui.weak(i18n.t(cx.disk_use_note));
    if let Some((key, params)) = &cx.max_time_clamped {
        ui.weak(note_text(i18n, key, params));
    }
    ui.weak(i18n.t("aggregate.options.comment_sent_note"));
}

fn note_text(i18n: &I18n, key: &str, params: &[(&'static str, String)]) -> String {
    let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    i18n.t_with(key, &params)
}

fn show_disk_use(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let label = i18n.t("aggregate.options.allow_disk_use.label");
    let picked = tri_combo(
        ui,
        cx.id_scope,
        "allow_disk_use",
        &label,
        form.allow_disk_use,
        i18n,
    );
    g.pick(picked, OptionsEdit::AllowDiskUse);
}

/// The time limit field, with the defaults as its placeholder.
fn show_max_time(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let (key, params) = &cx.max_time_placeholder;
    let typed = typed_field(
        ui,
        cx.id_scope,
        &i18n.t("aggregate.options.max_time.label"),
        &form.max_time,
        &note_text(i18n, key, params),
        problem(form, OptionField::MaxTime, i18n),
    );
    g.typed(typed, OptionsEdit::MaxTime);
}

fn show_comment(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
    let typed = typed_field(
        ui,
        cx.id_scope,
        &i18n.t("aggregate.options.comment.label"),
        &form.comment,
        &i18n.t(cx.comment_placeholder),
        problem(form, OptionField::Comment, i18n),
    );
    g.typed(typed, OptionsEdit::Comment);
}

fn show_read_preference(
    ui: &mut egui::Ui,
    form: &OptionsForm,
    g: &mut Gathered,
    cx: &FormContext<'_>,
    i18n: &I18n,
) {
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
    let picked = pick_combo(
        ui,
        cx.id_scope,
        "read_preference",
        &label,
        form.read_preference,
        &modes,
    );
    g.pick(picked, OptionsEdit::ReadPreference);
}

#[cfg(test)]
#[path = "options_form_tests.rs"]
mod tests;
