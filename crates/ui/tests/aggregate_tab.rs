//! The Aggregate tab renders a three-stage pipeline with a shown preview, and its main controls
//! are found by their accessible names. Fast, offline, no GPU. (The full accessibility pass of
//! the tab lives with its snapshots.)
use bson::doc;
use curlybrace_ui::I18n;
use curlybrace_ui::app::AggregateAction;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::aggregate::PreviewState;
use curlybrace_ui::viewmodel::aggregate_view::AggregateView;
use curlybrace_ui::views::aggregate::show_aggregate;
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

type Frame = (AggregateState, AggregateView, Vec<AggregateAction>);

fn t(key: &str) -> String {
    I18n::english().t(key)
}

/// `$match`, `$sort`, `$limit`, the last previewed with two documents.
fn three_stages_shown() -> AggregateState {
    let draft = AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
        .add_stage("$limit");
    let (draft, sent) = draft.begin_preview();
    let (request, _) = sent.expect("a valid pipeline is sent");
    draft.arrived(
        request,
        Ok(vec![doc! { "_id": 1, "a": 1 }, doc! { "_id": 2, "a": 2 }]),
    )
}

fn open(draft: AggregateState) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .build_ui_state(
            |ui, (draft, view, seen): &mut Frame| {
                seen.extend(show_aggregate(ui, draft, view, &I18n::english()));
            },
            (draft, AggregateView::default(), Vec::new()),
        );
    harness.run_steps(4);
    harness
}

#[test]
fn the_tab_renders_its_controls_by_name() {
    let draft = three_stages_shown();
    assert!(matches!(draft.preview, PreviewState::Shown(_)));
    let harness = open(draft);
    harness.get_by_label(&t("aggregate.add_stage"));
    harness.get_by_label(&t("aggregate.refresh"));
    harness.get_by_label(&t("aggregate.mode.stages"));
    harness.get_by_label(&t("aggregate.mode.text"));
    let i18n = I18n::english();
    harness.get_by_label(&i18n.t_with("aggregate.stage.move_up_n", &[("n", "2")]));
    harness.get_by_label(&i18n.t_with("aggregate.stage.remove_n", &[("n", "3")]));
    harness.get_by_label(&i18n.t_with("aggregate.stage.editor_n", &[("n", "1")]));
    harness.get_by_label(&t("aggregate.stage.previewed"));
    harness.get_by_label(&i18n.t_with("aggregate.preview.heading", &[("n", "3")]));
    // A shown preview asks for nothing on its own.
    assert!(harness.state().2.is_empty(), "{:?}", harness.state().2);
}

#[test]
fn an_idle_pipeline_asks_for_one_preview_on_first_display() {
    let draft = AggregateState::default().add_stage("$match");
    let mut harness = open(draft);
    harness.run_steps(4);
    assert_eq!(harness.state().2, [AggregateAction::Refresh]);
}

#[test]
fn clicking_move_up_of_stage_two_reports_it() {
    let mut harness = open(three_stages_shown());
    let name = I18n::english().t_with("aggregate.stage.move_up_n", &[("n", "2")]);
    harness.get_by_label(&name).click();
    harness.run_steps(2);
    assert_eq!(harness.state().2, [AggregateAction::MoveUp(1)]);
}
