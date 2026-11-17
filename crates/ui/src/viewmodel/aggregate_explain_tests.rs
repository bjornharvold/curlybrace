//! The explain state's ops: request numbering, stale answers, failures, the out-of-date check.
use super::*;
use bson::doc;
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::pipeline::PipelineOptions;

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

fn pipeline() -> Vec<Document> {
    vec![doc! { "$match": { "status": "open" } }]
}

fn loading() -> (ExplainUi, u64) {
    let (ui, request, _) = ExplainUi::default().begun(pipeline(), &PipelineOptions::default());
    (ui, request)
}

#[test]
fn a_run_is_numbered_shows_the_plan_tab_and_waits() {
    let ui = ExplainUi::default().with_verbosity(ExplainVerbosity::ExecutionStats);
    let (next, request, verbosity) = ui.begun(pipeline(), &PipelineOptions::default());
    assert_eq!(request, 1);
    assert_eq!(verbosity, ExplainVerbosity::ExecutionStats);
    assert_eq!(next.tab, ResultTab::Plan);
    assert!(next.is_loading());
    assert!(next.awaits(1));
    let (again, second, _) = next.begun(pipeline(), &PipelineOptions::default());
    assert_eq!(second, 2, "a newer run is a newer number");
    assert!(!again.awaits(1), "the older answer is stale now");
}

#[test]
fn the_awaited_answer_is_parsed_and_keeps_the_raw_document() {
    let (ui, request) = loading();
    let done = ui.arrived(request, Ok(explained(collscan())));
    let shown = done.done().expect("done");
    assert!(matches!(*shown.plan, AggPlanOutcome::Plan(_)));
    assert_eq!(*shown.raw, collscan());
    assert_eq!(&*shown.for_draft, pipeline().as_slice());
    assert_eq!(shown.verbosity, ExplainVerbosity::QueryPlanner);
}

#[test]
fn a_document_that_is_not_a_plan_is_not_understood_but_still_raw() {
    let (ui, request) = loading();
    let bad = doc! { "ok": 0, "errmsg": "no" };
    let done = ui.arrived(request, Ok(explained(bad.clone())));
    let shown = done.done().expect("done");
    assert_eq!(*shown.plan, AggPlanOutcome::NotUnderstood);
    assert_eq!(*shown.raw, bad);
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
    assert!(
        !ui.is_out_of_date(None, &PipelineOptions::default()),
        "nothing shown, nothing to mark"
    );
    let done = ui.arrived(request, Ok(explained(collscan())));
    assert!(!done.is_out_of_date(Some(&pipeline()), &PipelineOptions::default()));
    let edited = vec![doc! { "$match": { "status": "closed" } }];
    assert!(done.is_out_of_date(Some(&edited), &PipelineOptions::default()));
    assert!(
        done.is_out_of_date(None, &PipelineOptions::default()),
        "the draft no longer explains"
    );
}

#[test]
fn a_plan_is_out_of_date_once_the_measure_setting_differs_from_the_one_it_was_made_with() {
    let (ui, request) = loading();
    let done = ui.arrived(request, Ok(explained(collscan())));
    let same = pipeline();
    assert!(!done.is_out_of_date(Some(&same), &PipelineOptions::default()));
    let measured = done.with_verbosity(ExplainVerbosity::ExecutionStats);
    assert!(
        measured.is_out_of_date(Some(&same), &PipelineOptions::default()),
        "plan-only plan, measure now on"
    );
    let back = measured.with_verbosity(ExplainVerbosity::QueryPlanner);
    assert!(
        !back.is_out_of_date(Some(&same), &PipelineOptions::default()),
        "switching back makes it current again"
    );
}

#[test]
fn cloning_a_done_state_shares_the_plan_the_document_and_the_pipeline() {
    let (ui, request) = loading();
    let done = ui.arrived(request, Ok(explained(collscan())));
    let copy = done.clone();
    let (a, b) = (done.done().expect("done"), copy.done().expect("done"));
    assert!(Arc::ptr_eq(&a.plan, &b.plan));
    assert!(Arc::ptr_eq(&a.raw, &b.raw));
    assert!(Arc::ptr_eq(&a.for_draft, &b.for_draft));
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

#[test]
fn a_loading_state_prints_its_shape_never_the_pipeline() {
    let sentinel = "SENTINEL_9f3a";
    let secret = vec![doc! { "$match": { "email": sentinel } }];
    let (loading, request, _) = ExplainUi::default().begun(secret, &PipelineOptions::default());
    let shown = format!("{loading:?}");
    assert!(!shown.contains(sentinel), "{shown}");
    assert!(shown.contains("Loading"), "{shown}");
    let mut document = collscan();
    document.insert("note", sentinel);
    let done = loading.arrived(request, Ok(explained(document)));
    let shown = format!("{done:?}");
    assert!(!shown.contains(sentinel), "{shown}");
    assert!(shown.contains("Done"), "{shown}");
}

#[test]
fn a_failure_prints_its_kind_not_its_detail() {
    let (ui, request) = loading();
    let error = CoreError::InvalidDocument("SENTINEL_9f3a".into());
    let failed = ui.arrived(request, Err(error));
    let shown = format!("{failed:?}");
    assert!(!shown.contains("SENTINEL_9f3a"), "{shown}");
    assert!(shown.contains("Failed"), "{shown}");
}
