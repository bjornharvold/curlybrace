//! The explain state's ops: request numbering, stale answers, failures, the out-of-date check.
use super::*;
use bson::doc;
use curlybrace_core::ejson::editor_json_to_doc;

fn explained(document: Document) -> Explain {
    Explain {
        document,
        plan_only: true,
    }
}

fn collscan() -> Document {
    editor_json_to_doc(include_str!(
        "../../../core/tests/fixtures/explain_aggregate/stages_collscan.json"
    ))
    .expect("fixture parses")
}

fn pipeline() -> Fingerprint {
    vec![doc! { "$match": { "status": "open" } }]
}

fn loading() -> (ExplainUi, u64) {
    let (ui, request, _) = ExplainUi::default().begun(pipeline());
    (ui, request)
}

#[test]
fn a_run_is_numbered_shows_the_plan_tab_and_waits() {
    let ui = ExplainUi::default().with_verbosity(ExplainVerbosity::ExecutionStats);
    let (next, request, verbosity) = ui.begun(pipeline());
    assert_eq!(request, 1);
    assert_eq!(verbosity, ExplainVerbosity::ExecutionStats);
    assert_eq!(next.tab, ResultTab::Plan);
    assert!(next.is_loading());
    assert!(next.awaits(1));
    let (again, second, _) = next.begun(pipeline());
    assert_eq!(second, 2, "a newer run is a newer number");
    assert!(!again.awaits(1), "the older answer is stale now");
}

#[test]
fn the_awaited_answer_is_parsed_and_keeps_the_raw_document() {
    let (ui, request) = loading();
    let done = ui.arrived(request, Ok(explained(collscan())));
    let shown = done.done().expect("done");
    assert!(matches!(shown.plan, AggPlanOutcome::Plan(_)));
    assert_eq!(shown.raw, collscan());
    assert_eq!(shown.for_draft, pipeline());
    assert_eq!(shown.verbosity, ExplainVerbosity::QueryPlanner);
}

#[test]
fn a_document_that_is_not_a_plan_is_not_understood_but_still_raw() {
    let (ui, request) = loading();
    let bad = doc! { "ok": 0, "errmsg": "no" };
    let done = ui.arrived(request, Ok(explained(bad.clone())));
    let shown = done.done().expect("done");
    assert_eq!(shown.plan, AggPlanOutcome::NotUnderstood);
    assert_eq!(shown.raw, bad);
}

#[test]
fn a_stale_or_unawaited_answer_changes_nothing() {
    let (ui, request) = loading();
    let same = ui.clone().arrived(request + 1, Ok(explained(collscan())));
    assert_eq!(same, ui);
    let idle = ExplainUi::default();
    assert_eq!(idle.clone().arrived(1, Ok(explained(collscan()))), idle);
    let done = ui.arrived(request, Ok(explained(collscan())));
    let late = done.clone().arrived(request, Err(CoreError::Unreachable));
    assert_eq!(late, done, "an answer is taken once");
}

#[test]
fn a_failure_stays_in_the_pane_with_its_reason() {
    let (ui, request) = loading();
    let failed = ui.arrived(request, Err(CoreError::Database("boom".into())));
    assert_eq!(
        failed.state,
        ExplainState::Failed {
            reason: CoreError::Database("boom".into())
        }
    );
    assert_eq!(failed.tab, ResultTab::Plan);
}

#[test]
fn a_reset_forgets_the_plan_makes_answers_stale_and_keeps_the_choice_of_verbosity() {
    let (ui, request) = loading();
    let ui = ui.with_verbosity(ExplainVerbosity::ExecutionStats);
    let reset = ui.reset();
    assert_eq!(reset.state, ExplainState::Idle);
    assert_eq!(reset.tab, ResultTab::Preview);
    assert_eq!(reset.verbosity, ExplainVerbosity::ExecutionStats);
    let after = reset.clone().arrived(request, Ok(explained(collscan())));
    assert_eq!(after, reset);
    assert!(reset.request > request);
}

#[test]
fn giving_up_frees_only_the_awaited_request() {
    let (ui, request) = loading();
    assert_eq!(ui.clone().given_up(request + 5), ui);
    assert_eq!(ui.given_up(request).state, ExplainState::Idle);
}

#[test]
fn a_plan_is_out_of_date_once_the_pipeline_it_was_asked_for_differs() {
    let (ui, request) = loading();
    assert!(!ui.is_out_of_date(None), "nothing shown, nothing to mark");
    let done = ui.arrived(request, Ok(explained(collscan())));
    assert!(!done.is_out_of_date(Some(&pipeline())));
    let edited = vec![doc! { "$match": { "status": "closed" } }];
    assert!(done.is_out_of_date(Some(&edited)));
    assert!(done.is_out_of_date(None), "the draft no longer explains");
}

#[test]
fn debug_names_the_shape_and_never_the_document() {
    let (ui, request) = loading();
    let secret = doc! { "ok": 1, "private_field": "hunter2" };
    let done = ui.arrived(request, Ok(explained(secret)));
    let text = format!("{done:?}");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains("private_field"), "{text}");
}
