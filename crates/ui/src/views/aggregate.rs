//! The Aggregate tab: one wrapping header row (Add stage, Stages | Text, Refresh and the preview's
//! status), then the stage cards or the pipeline text, then the preview of the selected stage.
//! Draws the draft it is given and returns what the person did; the reducer decides what happens.
//! Typing in a stage is held in the view until it pauses, so a preview does not run per keystroke.
use super::aggregate_preview::show_preview;
use super::aggregate_stage::show_stages;
use super::aggregate_text::show_text;
use crate::I18n;
use crate::app::AggregateAction;
use crate::theme::{CONTROL_HEIGHT, Tone, chip, gate, segmented, tonal_button};
use crate::viewmodel::aggregate::{AggregateState, Mode, PreviewState, STAGE_MENU};
use crate::viewmodel::aggregate_view::{AggregateView, pipeline_error_message};
use curlybrace_core::pipeline::PipelineError;
use eframe::egui::{self, WidgetInfo, WidgetType};
use std::time::Duration;

/// The most of the tab's height the stage list or the text takes; the preview gets the rest.
const EDITOR_SHARE: f32 = 0.5;

/// How the stage list and the text scroll: wheel and bar, not dragging the content. A drag
/// area would also be an unnamed stop in the Tab order between the header and the first stage.
pub(super) const NO_DRAG_SCROLL: egui::scroll_area::ScrollSource =
    egui::scroll_area::ScrollSource {
        drag: false,
        ..egui::scroll_area::ScrollSource::ALL
    };

/// Draws the tab; returns the actions of this frame in the order they apply.
pub fn show_aggregate(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    view: &mut AggregateView,
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let header = show_header(ui, draft, i18n);
    let max_height = ui.available_height() * EDITOR_SHARE;
    let edited = match draft.mode {
        Mode::Stages => show_stages(ui, draft, view, max_height, i18n),
        Mode::Text => show_text(ui, draft, max_height, i18n),
    };
    ui.separator();
    show_preview(ui, draft, view, i18n);
    let actions = header.into_iter().chain(edited).collect();
    let actions = with_due_edit(ui, view, actions);
    with_auto_refresh(draft, view, actions)
}

/// `actions` led by the typed stage edit once the typing paused, or at once when another action
/// follows (it must apply to the typed text). While an edit waits, a repaint is asked for when it
/// falls due.
fn with_due_edit(
    ui: &egui::Ui,
    view: &mut AggregateView,
    actions: Vec<AggregateAction>,
) -> Vec<AggregateAction> {
    let now = ui.input(|i| i.time);
    let (next, due) = std::mem::take(view).take_due(now, !actions.is_empty());
    *view = next;
    if let Some(left) = view.due_in(now) {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(left));
    }
    due.map(|(index, text)| AggregateAction::Edit(index, text))
        .into_iter()
        .chain(actions)
        .collect()
}

/// The waiting stage edit as an `Edit`, taken at once: the tab is being left, and the draft
/// should hold what was typed rather than wait for the tab to be drawn again.
pub fn flush_typing(view: &mut AggregateView) -> Option<AggregateAction> {
    let (next, due) = std::mem::take(view).take_due(f64::INFINITY, true);
    *view = next;
    due.map(|(index, text)| AggregateAction::Edit(index, text))
}

/// A Refresh of an idle pane with stages to run (first display, another collection, a new
/// connection), asked once per request so a Refresh that cannot run is not repeated every frame.
fn with_auto_refresh(
    draft: &AggregateState,
    view: &mut AggregateView,
    actions: Vec<AggregateAction>,
) -> Vec<AggregateAction> {
    let quiet = actions.is_empty() && view.pending.is_none();
    if !quiet || !draft.wants_auto_refresh(view.auto_sent) {
        return actions;
    }
    view.auto_sent = Some(draft.request);
    vec![AggregateAction::Refresh]
}

/// Add stage, the mode toggle, Refresh and the preview's status chip in one wrapping row.
fn show_header(ui: &mut egui::Ui, draft: &AggregateState, i18n: &I18n) -> Vec<AggregateAction> {
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(CONTROL_HEIGHT);
        // A stage added in text mode would not be in the text being edited, and a Refresh there
        // would preview the stages the text is replacing.
        let stages = draft.mode == Mode::Stages;
        let added = add_stage_menu(ui, stages, i18n);
        let mode = mode_toggle(ui, draft.mode, i18n);
        let refresh = gate(ui, stages, |ui| {
            tonal_button(ui, i18n.t("aggregate.refresh"))
        });
        let refresh = refresh.clicked() && stages;
        let (tone, key) = status_chip(&draft.preview);
        chip(ui, tone, &i18n.t(key));
        [
            added.map(AggregateAction::AddStage),
            mode.map(AggregateAction::SetMode),
            refresh.then_some(AggregateAction::Refresh),
        ]
        .into_iter()
        .flatten()
        .collect()
    })
    .inner
}

/// The Add stage menu of `STAGE_MENU` names (operators, shown as they are written); the name
/// picked, if any.
fn add_stage_menu(ui: &mut egui::Ui, enabled: bool, i18n: &I18n) -> Option<String> {
    let label = i18n.t("aggregate.add_stage");
    let mut chosen = None;
    let combo = ui
        .add_enabled_ui(enabled, |ui| {
            ui.spacing_mut().interact_size.y = ui.spacing().interact_size.y.max(CONTROL_HEIGHT);
            egui::ComboBox::from_id_salt("aggregate_add_stage")
                .selected_text(label.clone())
                .show_ui(ui, |ui| {
                    for name in STAGE_MENU {
                        if ui.selectable_label(false, name).clicked() {
                            chosen = Some(name.to_string());
                        }
                    }
                })
        })
        .inner;
    combo
        .response
        .widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, enabled, label.clone()));
    chosen
}

/// Stages | Text; the mode picked when it differs from `mode`.
fn mode_toggle(ui: &mut egui::Ui, mode: Mode, i18n: &I18n) -> Option<Mode> {
    let items = [
        (Mode::Stages, i18n.t("aggregate.mode.stages")),
        (Mode::Text, i18n.t("aggregate.mode.text")),
    ];
    segmented(ui, &items, mode)
}

/// The tone and label key of the preview's status chip.
pub(super) fn status_chip(preview: &PreviewState) -> (Tone, &'static str) {
    match preview {
        PreviewState::Idle => (Tone::Neutral, "aggregate.status.idle"),
        PreviewState::Loading { .. } => (Tone::Neutral, "aggregate.status.loading"),
        PreviewState::Shown(_) => (Tone::Good, "aggregate.status.shown"),
        PreviewState::Failed { .. } => (Tone::Bad, "aggregate.status.failed"),
    }
}

/// The translated message of a pipeline error.
pub(super) fn pipeline_error_text(error: &PipelineError, i18n: &I18n) -> String {
    let (key, params) = pipeline_error_message(error);
    let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    i18n.t_with(key, &params)
}

/// A button of a stage card: short visible text, and an accessible name with the stage's
/// position (e.g. "Move stage 2 up"); whether it was clicked.
pub(super) fn named_button(ui: &mut egui::Ui, enabled: bool, (text, name): (&str, String)) -> bool {
    let response = gate(ui, enabled, |ui| {
        crate::theme::compact_text_button(ui, text)
    });
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name.clone()));
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodel::aggregate::Preview;

    #[test]
    fn every_status_has_its_own_label() {
        let shown = PreviewState::Shown(Preview {
            docs: Vec::new(),
            not_run: Vec::new(),
        });
        let loading = PreviewState::Loading {
            not_run: Vec::new(),
            last: None,
        };
        let keys: Vec<_> = [PreviewState::Idle, loading, shown]
            .iter()
            .map(|p| status_chip(p).1)
            .collect();
        assert_eq!(
            keys,
            [
                "aggregate.status.idle",
                "aggregate.status.loading",
                "aggregate.status.shown"
            ]
        );
    }

    #[test]
    fn an_idle_pane_asks_for_one_refresh_until_the_request_changes() {
        let draft = AggregateState::default().add_stage("$match");
        let mut view = AggregateView::default();
        assert_eq!(
            with_auto_refresh(&draft, &mut view, Vec::new()),
            [AggregateAction::Refresh]
        );
        assert!(with_auto_refresh(&draft, &mut view, Vec::new()).is_empty());
        let moved = draft.on_collection_changed();
        assert_eq!(
            with_auto_refresh(&moved, &mut view, Vec::new()),
            [AggregateAction::Refresh]
        );
    }

    #[test]
    fn leaving_the_tab_takes_the_waiting_edit() {
        let (mut view, _) = AggregateView::default().edited(2, "x".into(), 1.0);
        assert_eq!(
            flush_typing(&mut view),
            Some(AggregateAction::Edit(2, "x".into()))
        );
        assert_eq!(view.pending, None);
        assert_eq!(flush_typing(&mut view), None);
    }

    #[test]
    fn an_action_of_the_person_comes_before_any_automatic_refresh() {
        let draft = AggregateState::default().add_stage("$match");
        let mut view = AggregateView::default();
        let picked = vec![AggregateAction::Select(0)];
        assert_eq!(with_auto_refresh(&draft, &mut view, picked.clone()), picked);
        assert_eq!(view.auto_sent, None, "it may still ask later");
    }
}
