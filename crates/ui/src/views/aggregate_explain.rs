//! The result pane of the Aggregate tab under the stages: the tabs Preview | Plan | Raw, the
//! Explain button, and the Plan and Raw tabs. The Plan tab asks for the server's plan (plan only
//! by default; a toggle runs the pipeline for counters and says so), shows it as a tree with
//! hints, and marks it out of date once the pipeline is edited. Draws what it is given and returns
//! what the person did; the reducer decides what happens.
use super::aggregate::pipeline_error_text;
use super::aggregate_plan::show_plan;
use super::aggregate_preview::{show_preview_body, show_preview_heading};
use super::error_label::show_error;
use super::json::show_json;
use crate::I18n;
use crate::app::{AggregateAction, ExplainAction};
use crate::theme::{CONTROL_HEIGHT, Tone, chip, gate, segmented, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_explain::{ExplainDone, ExplainState, ResultTab};
use crate::viewmodel::aggregate_view::AggregateView;
use crate::viewmodel::code_export::CodeBlock;
use curlybrace_core::ejson::doc_to_view_json;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{AGGREGATE_EXPLAIN_MAX_TIME_MS, AggPlanOutcome, ExplainVerbosity};
use curlybrace_core::pipeline::Stage;
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use eframe::egui;

/// Why the Explain button is off, in words; `None` when it is on. `typed` are the stages with the
/// text still being typed applied. The pipeline check is remembered between repaints (see
/// `viewmodel::aggregate_explain_block`).
pub fn explain_blocked(
    draft: &AggregateState,
    typed: &[Stage],
    view: &AggregateView,
    i18n: &I18n,
) -> Option<String> {
    match &*view.explain_request(draft, typed) {
        Err(CodeBlock::Pipeline(error)) => Some(pipeline_error_text(error, i18n)),
        Err(CodeBlock::TextNotApplied) => {
            Some(i18n.t("aggregate.explain.disabled.text_not_applied"))
        }
        Ok(_) if draft.explain.is_loading() => Some(i18n.t("aggregate.explain.disabled.running")),
        Ok(_) => None,
    }
}

/// The Explain button; true when it was clicked while on. `blocked` is the reason it is off,
/// shown as its tooltip and its accessible description.
pub fn show_explain_button(ui: &mut egui::Ui, blocked: Option<&str>, i18n: &I18n) -> bool {
    let label = i18n.t("aggregate.explain.button");
    let response = match blocked {
        None => tonal_button(ui, label).on_hover_text(i18n.t("aggregate.explain.button_hint")),
        Some(reason) => {
            let off = gate(ui, false, |ui| tonal_button(ui, label)).on_hover_text(reason);
            let why = reason.to_string();
            ui.ctx()
                .accesskit_node_builder(off.id, |node| node.set_description(why));
            off
        }
    };
    response.clicked() && blocked.is_none()
}

/// The tabs and the pane of the one in view; the actions the person took.
pub(super) fn show_results(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    view: &mut AggregateView,
    typed: &[Stage],
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let tab = draft.explain.tab;
    let current = view.explain_request(draft, typed);
    let current_pipeline = current.as_ref().as_ref().ok().map(Vec::as_slice);
    let out_of_date = draft.explain.is_out_of_date(current_pipeline);
    let mut actions = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(CONTROL_HEIGHT);
        if let Some(picked) = show_tabs(ui, tab, i18n) {
            actions.push(select(picked));
        }
        if out_of_date && tab != ResultTab::Preview {
            let mark = chip(ui, Tone::Warning, &i18n.t("aggregate.explain.out_of_date"));
            mark.on_hover_text(i18n.t("aggregate.explain.out_of_date_hint"));
        }
        // The preview's own heading shares the row, so the tabs cost the documents no height.
        if tab == ResultTab::Preview {
            show_preview_heading(ui, draft, view, i18n);
        }
    });
    match tab {
        ResultTab::Preview => show_preview_body(ui, draft, view, i18n),
        ResultTab::Plan => actions.extend(show_plan_tab(ui, draft, i18n)),
        ResultTab::Raw => actions.extend(show_raw_tab(ui, draft, view, i18n)),
    }
    actions
}

fn select(tab: ResultTab) -> AggregateAction {
    AggregateAction::Explain(ExplainAction::SelectTab(tab))
}

/// Preview | Plan | Raw as one radio group; the tab picked when it differs from `tab`.
fn show_tabs(ui: &mut egui::Ui, tab: ResultTab, i18n: &I18n) -> Option<ResultTab> {
    let items = [
        (ResultTab::Preview, i18n.t("aggregate.explain.tabs.preview")),
        (ResultTab::Plan, i18n.t("aggregate.explain.tabs.plan")),
        (ResultTab::Raw, i18n.t("aggregate.explain.tabs.raw")),
    ];
    segmented(ui, &items, tab)
}

/// The verbosity toggle with the cost note beside it, and the plan or what stands in its place.
fn show_plan_tab(ui: &mut egui::Ui, draft: &AggregateState, i18n: &I18n) -> Vec<AggregateAction> {
    let explain = &draft.explain;
    let mut actions = Vec::new();
    let mut measure = explain.verbosity == ExplainVerbosity::ExecutionStats;
    let label = i18n.t("aggregate.explain.verbosity");
    ui.horizontal_wrapped(|ui| {
        let toggle = ui.checkbox(&mut measure, label);
        if toggle.changed() {
            actions.push(AggregateAction::Explain(ExplainAction::SetVerbosity(
                verbosity_of(measure),
            )));
        }
        if measure {
            ui.weak(cost_note(i18n));
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("explain_plan")
        .auto_shrink([false, true])
        .show(ui, |ui| show_state(ui, &explain.state, i18n));
    actions
}

fn verbosity_of(measure: bool) -> ExplainVerbosity {
    match measure {
        true => ExplainVerbosity::ExecutionStats,
        false => ExplainVerbosity::QueryPlanner,
    }
}

/// The note that measuring runs the pipeline on the server, with the time it is cut off at.
fn cost_note(i18n: &I18n) -> String {
    let seconds = (AGGREGATE_EXPLAIN_MAX_TIME_MS / 1000).to_string();
    i18n.t_with("aggregate.explain.cost_note", &[("seconds", &seconds)])
}

fn show_state(ui: &mut egui::Ui, state: &ExplainState, i18n: &I18n) {
    match state {
        ExplainState::Idle => {
            ui.label(i18n.t("aggregate.explain.idle"));
        }
        ExplainState::Loading { .. } => {
            ui.label(i18n.t("aggregate.explain.loading"));
        }
        ExplainState::Failed { reason } => show_failure(ui, reason, i18n),
        ExplainState::Done(done) => match &*done.plan {
            AggPlanOutcome::Plan(plan) => show_plan(ui, plan, i18n),
            AggPlanOutcome::NotUnderstood => {
                ui.label(i18n.t("aggregate.explain.not_understood"));
            }
        },
    }
}

/// Why no plan came: the timeout has words of its own, any other error is shown as such.
fn show_failure(ui: &mut egui::Ui, reason: &CoreError, i18n: &I18n) {
    match reason {
        CoreError::Database(text) if text == COMMAND_TIMEOUT_MESSAGE => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                i18n.t("aggregate.explain.error.timeout"),
            );
        }
        other => show_error(ui, other, i18n),
    }
}

/// The document as the server sent it, in the same read-only JSON view as the preview, with a
/// button that copies it.
fn show_raw_tab(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    view: &mut AggregateView,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let Some(done) = draft.explain.done() else {
        ui.label(i18n.t("aggregate.explain.raw.none"));
        return Vec::new();
    };
    let copy = ui
        .horizontal_wrapped(|ui| tonal_button(ui, i18n.t("aggregate.explain.raw.copy")))
        .inner
        .clicked();
    if copy {
        ui.ctx().copy_text(doc_to_view_json(&done.raw));
    }
    show_raw_json(ui, done, view, i18n);
    copy.then_some(AggregateAction::Explain(ExplainAction::CopyRaw))
        .into_iter()
        .collect()
}

fn show_raw_json(ui: &mut egui::Ui, done: &ExplainDone, view: &mut AggregateView, i18n: &I18n) {
    let AggregateView {
        raw_tree, raw_page, ..
    } = view;
    let (page, key) = raw_page.of_shared(&done.raw);
    raw_tree.synced_to(&key);
    raw_tree.read_only = true;
    raw_tree.projected = true;
    raw_tree.copy_blocked = true;
    raw_tree.no_row_menu = true;
    // Read-only and without a row menu: the view reports no choice.
    let _offers_nothing = show_json(ui, page, &key, raw_tree, i18n);
}

#[cfg(test)]
#[path = "aggregate_explain_tests.rs"]
mod tests;
