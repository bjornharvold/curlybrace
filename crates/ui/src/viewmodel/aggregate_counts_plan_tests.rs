use super::*;
use bson::doc;

fn draft() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$group")
        .add_stage("$sort")
}

fn plan(d: &AggregateState) -> CountPlanOutcome {
    count_plan(d, &d.stages)
}

fn items(d: &AggregateState) -> Vec<PlanItem> {
    plan(d).expect("a plan").items
}

fn skips(items: &[PlanItem]) -> Vec<(usize, NotCounted)> {
    items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Skip { index, reason } => Some((*index, *reason)),
            PlanItem::Send { .. } => None,
        })
        .collect()
}

#[test]
fn every_enabled_stage_gets_its_own_bounded_pipeline() {
    let d = draft();
    let items = items(&d);
    assert_eq!(items.len(), 3);
    let PlanItem::Send { index, pipeline } = &items[1] else {
        panic!("stage 1 is sent");
    };
    assert_eq!(*index, 1);
    assert_eq!(pipeline.len(), 4, "two stages, a limit and a count");
    assert_eq!(pipeline[2], doc! { "$limit": COUNT_CAP });
    assert_eq!(pipeline[3], doc! { "$count": "n" });
}

#[test]
fn disabled_stages_are_left_out_and_never_part_of_a_count() {
    let d = draft().toggle(1);
    let items = items(&d);
    let indexes: Vec<usize> = items
        .iter()
        .map(|i| match i {
            PlanItem::Send { index, .. } | PlanItem::Skip { index, .. } => *index,
        })
        .collect();
    assert_eq!(indexes, [0, 2]);
    let PlanItem::Send { pipeline, .. } = &items[1] else {
        panic!("sent");
    };
    assert_eq!(pipeline.len(), 4, "$match, $sort, limit, count");
}

#[test]
fn a_stage_that_does_not_parse_stops_the_stages_that_need_it() {
    let d = draft().edit(1, "{ not json");
    let items = items(&d);
    assert!(matches!(items[0], PlanItem::Send { index: 0, .. }));
    assert_eq!(
        skips(&items),
        [(1, NotCounted::Invalid), (2, NotCounted::AfterInvalid)]
    );
}

#[test]
fn a_write_stage_and_everything_after_it_is_not_counted() {
    let d = draft().edit(1, "{ \"$out\": \"x\" }");
    let items = items(&d);
    assert!(matches!(items[0], PlanItem::Send { index: 0, .. }));
    assert_eq!(
        skips(&items),
        [(1, NotCounted::WriteStage), (2, NotCounted::WriteStage)]
    );
}

#[test]
fn a_write_stage_nested_in_a_sub_pipeline_is_never_sent() {
    let nested = "{ \"$lookup\": { \"from\": \"a\", \"pipeline\": [ { \"$out\": \"x\" } ], \"as\": \"b\" } }";
    let d = draft().edit(1, nested);
    let items = items(&d);
    assert_eq!(
        skips(&items),
        [(1, NotCounted::NestedWrite), (2, NotCounted::NestedWrite)]
    );
}

#[test]
fn nothing_enabled_or_nothing_countable_blocks_with_a_reason() {
    assert_eq!(
        plan(&AggregateState::default()),
        Err(CountsBlock::Pipeline(PipelineError::Empty))
    );
    let off = draft().toggle(0).toggle(1).toggle(2);
    assert_eq!(plan(&off), Err(CountsBlock::Pipeline(PipelineError::Empty)));
    let broken = AggregateState::default()
        .add_stage("$match")
        .edit(0, "{ not json");
    assert_eq!(plan(&broken), Err(CountsBlock::Nothing));
}

#[test]
fn edited_text_that_is_not_applied_blocks_the_plan() {
    let d = draft().to_text().with_text("[]");
    assert_eq!(plan(&d), Err(CountsBlock::TextNotApplied));
    assert_eq!(plan(&draft().to_text()), Err(CountsBlock::TextMode));
}

#[test]
fn a_repaint_with_nothing_changed_parses_nothing_and_a_change_asks_again() {
    let view = AggregateView::default();
    let d = draft();
    let first = view.count_plan(&d, &d.stages);
    let second = view.count_plan(&d, &d.stages);
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(view.count_plan_evaluations(), 1);
    let selected = d.clone().select(0);
    view.count_plan(&selected, &selected.stages);
    assert_eq!(view.count_plan_evaluations(), 1, "selection is no input");
    let edited = selected.edit(0, "{ \"$match\": { \"a\": 1 } }");
    view.count_plan(&edited, &edited.stages);
    assert_eq!(view.count_plan_evaluations(), 2);
}

#[test]
fn text_mode_is_not_counted_even_when_its_text_is_applied() {
    let d = draft().to_text();
    assert_eq!(plan(&d), Err(CountsBlock::TextMode));
}

#[test]
fn a_write_stage_first_is_reported_as_a_write_stage_not_as_invalid() {
    let d = AggregateState::default()
        .add_stage("$match")
        .edit(0, "{ \"$out\": \"x\" }");
    assert_eq!(
        reason_of(
            &count_pipelines(&d.stages, COUNT_CAP)[0]
                .clone()
                .unwrap_err(),
            0
        ),
        NotCounted::WriteStage
    );
    assert_eq!(plan(&d), Err(CountsBlock::Nothing));
}
