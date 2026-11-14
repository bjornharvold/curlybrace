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
use egui_kittest::kittest::{NodeT, Queryable};

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
    open_stepping(draft, 0.25)
}

/// The tab with each frame `step_dt` seconds after the one before.
fn open_stepping(draft: AggregateState, step_dt: f32) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .with_step_dt(step_dt)
        .build_ui_state(
            |ui, (draft, view, seen): &mut Frame| {
                seen.extend(show_aggregate(ui, draft, view, &[], &I18n::english()));
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

/// Types `text` at the end of stage `n`'s editor (1-based).
fn type_in_stage(harness: &mut Harness<'static, Frame>, n: &str, text: &str) {
    let name = I18n::english().t_with("aggregate.stage.editor_n", &[("n", n)]);
    harness.get_by_label(&name).focus();
    harness.run_steps(1);
    harness.key_press(egui::Key::End);
    harness.get_by_label(&name).type_text(text);
    harness.run_steps(1);
}

#[test]
fn typing_in_a_second_stage_before_the_pause_still_sends_the_first_edit() {
    // Frames 10 ms apart: both stages are typed in well within the pause.
    let mut harness = open_stepping(three_stages_shown(), 0.01);
    type_in_stage(&mut harness, "1", " ");
    assert!(
        harness.state().2.is_empty(),
        "the first edit waits for the pause"
    );
    type_in_stage(&mut harness, "2", " ");
    let edited: Vec<usize> = harness
        .state()
        .2
        .iter()
        .filter_map(|a| match a {
            AggregateAction::Edit(i, _) => Some(*i),
            _ => None,
        })
        .collect();
    assert!(edited.contains(&0), "stage 1 edit is sent: {edited:?}");
}

#[test]
fn switching_to_text_mode_sends_the_waiting_edit_first() {
    let mut harness = open_stepping(three_stages_shown(), 0.01);
    type_in_stage(&mut harness, "1", " ");
    harness.get_by_label(&t("aggregate.mode.text")).click();
    harness.run_steps(1);
    let seen = &harness.state().2;
    assert!(
        matches!(
            seen.as_slice(),
            [AggregateAction::Edit(0, _), AggregateAction::SetMode(_)]
        ),
        "{seen:?}"
    );
}

#[test]
fn text_mode_warns_that_disabled_stages_are_not_kept_only_when_one_is() {
    let warning = t("aggregate.text.disabled_lost");
    let harness = open(three_stages_shown().to_text());
    assert!(harness.query_by_label(&warning).is_none());
    let harness = open(three_stages_shown().toggle(1).to_text());
    harness.get_by_label(&warning);
}

#[test]
fn refresh_is_off_in_text_mode_where_it_would_preview_the_old_stages() {
    let refresh = t("aggregate.refresh");
    let harness = open(three_stages_shown());
    assert!(
        !harness
            .get_by_label(&refresh)
            .accesskit_node()
            .is_disabled()
    );
    let harness = open(three_stages_shown().to_text());
    assert!(
        harness
            .get_by_label(&refresh)
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn a_disabled_selected_stage_is_not_headed_as_the_output_shown() {
    let i18n = I18n::english();
    let heading = i18n.t_with("aggregate.preview.heading", &[("n", "3")]);
    let harness = open(three_stages_shown().toggle(2));
    assert!(harness.query_by_label(&heading).is_none());
    // Once on the card's status line, once over the preview.
    assert_eq!(
        harness
            .query_all_by_label(&t("aggregate.stage.disabled"))
            .count(),
        2
    );
}
