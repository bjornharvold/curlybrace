//! The result pane's tabs, the Plan and Raw tabs and the Explain button: named controls,
//! keyboard operable, every state of the explain in words.
use super::*;
use crate::theme;
use crate::viewmodel::aggregate_explain::ExplainUi;
use crate::viewmodel::aggregate_explain_block::explain_request;
use bson::{Document, doc};
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::pipeline::PipelineOptions;
use curlybrace_core::store::Explain;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::NodeT;
use egui_kittest::kittest::Queryable;

struct Frame {
    draft: AggregateState,
    view: AggregateView,
    seen: Vec<AggregateAction>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn collscan() -> Document {
    editor_json_to_doc(include_str!(
        "../../../core/tests/fixtures/explain_aggregate/stages_collscan.json"
    ))
    .expect("fixture parses")
}

fn base() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$group")
        .select(1)
}

/// `draft` with an explain of its own pipeline answered with `document`.
fn explained(draft: AggregateState, document: Document) -> AggregateState {
    let pipeline = explain_request(&draft, &draft.stages).expect("valid");
    let (ui, request, _) = draft
        .explain
        .clone()
        .begun(pipeline, &PipelineOptions::default());
    let ui = ui.arrived(
        request,
        Ok(Explain {
            document,
            plan_only: true,
        }),
    );
    let mut draft = draft;
    draft.explain = ui;
    draft
}

fn open(draft: AggregateState) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 500.0))
        .build_ui_state(
            |ui, frame: &mut Frame| {
                let typed = frame.view.typed_stages(&frame.draft).into_owned();
                let actions =
                    show_results(ui, &frame.draft, &mut frame.view, &typed, &I18n::english());
                frame.seen.extend(actions);
            },
            Frame {
                draft,
                view: AggregateView::default(),
                seen: Vec::new(),
            },
        );
    theme::install(&harness.ctx);
    harness.run_steps(4);
    harness
}

fn on_tab(draft: AggregateState, tab: ResultTab) -> AggregateState {
    let mut draft = draft;
    draft.explain = draft.explain.with_tab(tab);
    draft
}

fn toggled(harness: &Harness<'static, Frame>, role: Role, label: &str) -> Option<Toggled> {
    harness
        .get_by_role_and_label(role, label)
        .accesskit_node()
        .toggled()
}

#[test]
fn the_tabs_are_named_radio_buttons_with_the_one_in_view_selected() {
    let harness = open(base());
    for (tab, key) in [
        (ResultTab::Preview, "aggregate.explain.tabs.preview"),
        (ResultTab::Plan, "aggregate.explain.tabs.plan"),
        (ResultTab::Raw, "aggregate.explain.tabs.raw"),
    ] {
        let expected = match tab == ResultTab::Preview {
            true => Toggled::True,
            false => Toggled::False,
        };
        assert_eq!(
            toggled(&harness, Role::RadioButton, &t(key)),
            Some(expected),
            "{key}"
        );
    }
}

#[test]
fn clicking_or_pressing_enter_on_a_tab_selects_it() {
    let mut harness = open(base());
    harness
        .get_by_role_and_label(Role::RadioButton, &t("aggregate.explain.tabs.plan"))
        .click();
    harness.run_steps(2);
    let pick = AggregateAction::Explain(ExplainAction::SelectTab(ResultTab::Plan));
    assert!(harness.state().seen.contains(&pick));

    let mut keyboard = open(base());
    keyboard
        .get_by_role_and_label(Role::RadioButton, &t("aggregate.explain.tabs.raw"))
        .focus();
    keyboard.run_steps(2);
    keyboard.key_press(egui::Key::Enter);
    keyboard.run_steps(2);
    let raw = AggregateAction::Explain(ExplainAction::SelectTab(ResultTab::Raw));
    assert!(keyboard.state().seen.contains(&raw));
}

#[test]
fn an_idle_plan_tab_invites_an_explain_and_names_its_toggle() {
    let harness = open(on_tab(base(), ResultTab::Plan));
    harness.get_by_label(&t("aggregate.explain.idle"));
    assert_eq!(
        toggled(&harness, Role::CheckBox, &t("aggregate.explain.verbosity")),
        Some(Toggled::False)
    );
    let note = I18n::english().t_with("aggregate.explain.cost_note", &[("seconds", "10")]);
    assert!(
        harness.query_by_label(&note).is_none(),
        "no cost without it"
    );
}

#[test]
fn measuring_shows_the_cost_note_beside_its_toggle_and_toggling_asks_for_it() {
    let mut draft = on_tab(base(), ResultTab::Plan);
    draft.explain = draft
        .explain
        .with_verbosity(ExplainVerbosity::ExecutionStats);
    let mut harness = open(draft);
    let note = I18n::english().t_with("aggregate.explain.cost_note", &[("seconds", "10")]);
    harness.get_by_label(&note);
    harness
        .get_by_role_and_label(Role::CheckBox, &t("aggregate.explain.verbosity"))
        .click();
    harness.run_steps(2);
    let off = AggregateAction::Explain(ExplainAction::SetVerbosity(ExplainVerbosity::QueryPlanner));
    assert!(harness.state().seen.contains(&off));
}

#[test]
fn a_plan_is_drawn_and_marked_out_of_date_only_once_the_draft_changed() {
    let draft = on_tab(explained(base(), collscan()), ResultTab::Plan);
    let mark = t("aggregate.explain.out_of_date");
    let harness = open(draft.clone());
    harness.get_by_label(&t("aggregate.explain.hint.collection_scan"));
    assert!(harness.query_by_label(&mark).is_none());

    let edited = draft.edit(0, "{ \"$match\": { \"a\": 1 } }");
    let harness = open(edited);
    harness.get_by_label(&mark);
    harness.get_by_label(&t("aggregate.explain.hint.collection_scan"));
}

#[test]
fn a_document_that_is_not_a_plan_points_to_the_raw_tab() {
    let draft = on_tab(explained(base(), doc! { "ok": 0 }), ResultTab::Plan);
    let harness = open(draft);
    harness.get_by_label(&t("aggregate.explain.not_understood"));
}

#[test]
fn waiting_and_failing_have_words_inline() {
    let mut loading = on_tab(base(), ResultTab::Plan);
    let pipeline = vec![doc! { "$match": {} }];
    loading.explain = loading
        .explain
        .begun(pipeline, &PipelineOptions::default())
        .0;
    open(loading).get_by_label(&t("aggregate.explain.loading"));

    for (error, text) in [
        (
            CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into()),
            t("aggregate.explain.error.timeout"),
        ),
        (
            CoreError::ReadOnly,
            I18n::english().error(&CoreError::ReadOnly),
        ),
    ] {
        let mut draft = on_tab(base(), ResultTab::Plan);
        let (ui, request, _) = ExplainUi::default().begun(vec![], &PipelineOptions::default());
        draft.explain = ui.arrived(request, Err(error)).with_tab(ResultTab::Plan);
        open(draft).get_by_label(&text);
    }
}

#[test]
fn the_raw_tab_shows_the_document_and_offers_to_copy_it() {
    let draft = on_tab(explained(base(), collscan()), ResultTab::Raw);
    let mut harness = open(draft);
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.explain.raw.copy"))
        .click();
    harness.run_steps(2);
    let copied = AggregateAction::Explain(ExplainAction::CopyRaw);
    assert!(harness.state().seen.contains(&copied));
}

#[test]
fn repainting_the_raw_tab_does_not_rebuild_its_page() {
    let draft = on_tab(explained(base(), collscan()), ResultTab::Raw);
    let mut harness = open(draft);
    harness.run_steps(5);
    assert_eq!(harness.state().view.raw_page.builds(), 1);
}

#[test]
fn a_plan_made_with_another_measure_setting_is_marked_out_of_date() {
    let mark = t("aggregate.explain.out_of_date");
    let mut draft = on_tab(explained(base(), collscan()), ResultTab::Plan);
    assert!(open(draft.clone()).query_by_label(&mark).is_none());
    draft.explain = draft
        .explain
        .with_verbosity(ExplainVerbosity::ExecutionStats);
    let harness = open(draft);
    harness.get_by_label(&mark);
}

#[test]
fn the_raw_tab_without_an_explain_says_so() {
    open(on_tab(base(), ResultTab::Raw)).get_by_label(&t("aggregate.explain.raw.none"));
}

#[test]
fn the_explain_button_is_off_with_the_reason_named_as_its_description() {
    let reason = "no stage to explain";
    let mut harness = Harness::builder()
        .with_size(egui::vec2(400.0, 100.0))
        .build_ui(|ui| {
            show_explain_button(ui, Some(reason), &I18n::english());
        });
    theme::install(&harness.ctx);
    harness.run_steps(3);
    let name = t("aggregate.explain.button");
    let button = harness.get_by_role_and_label(Role::Button, &name);
    assert!(button.accesskit_node().is_disabled());
    assert_eq!(
        button.accesskit_node().description().as_deref(),
        Some(reason)
    );
}

#[test]
fn the_explain_button_is_blocked_with_the_previews_reasons_and_while_one_runs() {
    let i18n = I18n::english();
    let view = AggregateView::default();
    let none = AggregateState::default();
    assert_eq!(
        explain_blocked(&none, &none.stages, &view, &i18n),
        Some(pipeline_error_text(
            &curlybrace_core::pipeline::PipelineError::Empty,
            &i18n
        ))
    );
    let ok = base();
    assert_eq!(explain_blocked(&ok, &ok.stages, &view, &i18n), None);
    let mut running = ok.clone();
    running.explain = running.explain.begun(vec![], &PipelineOptions::default()).0;
    assert_eq!(
        explain_blocked(&running, &running.stages, &view, &i18n),
        Some(t("aggregate.explain.disabled.running"))
    );
    let edited = ok.clone().to_text().with_text("[]");
    assert_eq!(
        explain_blocked(&edited, &edited.stages, &view, &i18n),
        Some(t("aggregate.explain.disabled.text_not_applied"))
    );
}
