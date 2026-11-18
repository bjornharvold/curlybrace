//! The Options section of the Aggregate tab: the header toggle with its "Options (N)" indicator
//! and the section itself (collation on the left, the rest on the right above 900 pt, one column
//! below). Draws what it is given and returns what the person did; typed text is applied by the
//! caller (Enter, focus loss or a pause), the picks apply at once in the reducer.
use super::options_form::{FlagsWhenOff, FormContext, FormLayout, show_options_form};
use crate::I18n;
use crate::app::{AggregateAction, OptionsAction};
use crate::theme::shape::{self, radius};
use crate::theme::{current_ui, filled_button, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use curlybrace_core::pipeline::{
    AGGREGATE_RUN_MAX_TIME_MS, MaxTimeEffect, Operation, PREVIEW_MAX_TIME_MS,
};
use eframe::egui::{self, WidgetInfo, WidgetType};

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
    let cx = form_context(draft);
    let gathered = section_frame(ui)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            show_options_form(
                ui,
                &draft.options_ui.form,
                &draft.options_ui.hint,
                &cx,
                i18n,
            )
        })
        .inner;
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
    let retry = gathered
        .retry_hints
        .then_some(AggregateAction::Options(OptionsAction::HintListRetry));
    wanted
        .into_iter()
        .chain(retry)
        .chain(edits)
        .chain(apply)
        .chain(reset)
        .collect()
}

/// What the Aggregate tab tells the shared form: its own notes, and the time limits of a preview
/// and a run.
fn form_context(draft: &AggregateState) -> FormContext<'static> {
    let preview = (PREVIEW_MAX_TIME_MS / 1000).to_string();
    let run = (AGGREGATE_RUN_MAX_TIME_MS / 1000).to_string();
    let capped = draft.options.max_time_effect(Operation::Preview) == MaxTimeEffect::Capped;
    FormContext {
        id_scope: "aggregate",
        layout: FormLayout::Columns,
        disk_use_note: "aggregate.options.allow_disk_use.preview_note",
        comment_placeholder: "aggregate.options.comment.placeholder",
        max_time_placeholder: (
            "aggregate.options.max_time.placeholder",
            vec![("preview", preview.clone()), ("run", run)],
        ),
        max_time_clamped: capped.then(|| {
            (
                "aggregate.options.max_time.clamped",
                vec![("limit", preview)],
            )
        }),
        flags_when_off: FlagsWhenOff::Disabled,
    }
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

#[cfg(test)]
#[path = "aggregate_options_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "aggregate_options_status_tests.rs"]
mod status_tests;
