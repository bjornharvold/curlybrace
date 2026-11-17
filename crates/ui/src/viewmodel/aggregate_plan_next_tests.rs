use super::*;
use bson::{Document, doc};
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::pipeline_options::{CollationOption, HintOption, PipelineOptions, Strength};
use curlybrace_core::store::Explain;

fn fixture(name: &str) -> Document {
    let text = match name {
        "collscan" => {
            include_str!("../../../core/tests/fixtures/explain_aggregate/stages_collscan.json")
        }
        "ixscan" => {
            include_str!("../../../core/tests/fixtures/explain_aggregate/stages_ixscan.json")
        }
        "sort" => include_str!(
            "../../../core/tests/fixtures/explain_aggregate/real_limit_sort_match_sort.json"
        ),
        other => panic!("unknown fixture {other}"),
    };
    editor_json_to_doc(text).expect("fixture parses")
}

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn pipeline() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "A" } },
        doc! { "$sort": { "total": -1_i32 } },
    ]
}

/// A draft whose Plan tab holds the plan of `fixture(name)`, explained for `pipeline()`.
fn explained(name: &str, options: PipelineOptions) -> AggregateState {
    let draft = AggregateState::default();
    let (ui, request, _) = draft.explain.clone().begun(pipeline(), &options);
    let ui = ui.arrived(
        request,
        Ok(Explain {
            document: fixture(name),
            plan_only: true,
        }),
    );
    AggregateState {
        explain: ui,
        options,
        ..draft
    }
}

fn collation(locale: &str) -> CollationOption {
    CollationOption {
        locale: locale.into(),
        strength: Some(Strength::Secondary),
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    }
}

#[test]
fn nothing_before_a_plan() {
    let next = plan_next(&AggregateState::default(), Some(&ns()), true);
    assert_eq!(
        next,
        PlanNext {
            try_index: false,
            current_hint: None,
            blocked: None,
            suggested: None,
            command: None,
        }
    );
}

#[test]
fn a_collection_scan_offers_try_and_the_suggestion() {
    let next = plan_next(
        &explained("collscan", PipelineOptions::default()),
        Some(&ns()),
        true,
    );
    assert!(next.try_index);
    assert_eq!(next.blocked, None);
    let index = next.suggested.expect("a suggestion");
    assert_eq!(
        index.keys,
        vec![("status".to_string(), 1), ("total".to_string(), -1)]
    );
    assert_eq!(next.command, Some(index.command(&ns())));
}

#[test]
fn an_in_memory_sort_offers_them_too() {
    let draft = explained("sort", PipelineOptions::default());
    let done = draft.explain.done().expect("done");
    let AggPlanOutcome::Plan(plan) = &*done.plan else {
        panic!("a plan");
    };
    assert!(plan_wants_an_index(plan), "{:?}", plan.hints);
    let next = plan_next(&draft, Some(&ns()), true);
    assert!(next.try_index);
    assert!(next.suggested.is_some());
}

#[test]
fn an_index_scan_offers_nothing_unless_a_hint_is_set() {
    let next = plan_next(
        &explained("ixscan", PipelineOptions::default()),
        Some(&ns()),
        true,
    );
    assert!(!next.try_index);
    assert_eq!(next.suggested, None);
    assert_eq!(next.command, None);
    let hinted = PipelineOptions {
        hint: Some(HintOption::Name("status_1".into())),
        ..PipelineOptions::default()
    };
    let next = plan_next(&explained("ixscan", hinted), Some(&ns()), true);
    assert!(next.try_index, "Clear hint must stay reachable");
    assert_eq!(next.current_hint.as_deref(), Some("status_1"));
    assert_eq!(next.suggested, None, "the plan does not want an index");
}

#[test]
fn the_suggestion_comes_from_the_explained_pipeline_not_the_draft() {
    let draft = explained("collscan", PipelineOptions::default());
    let edited = draft
        .clone()
        .add_stage("$match")
        .edit(0, r#"{ "$match": { "other": 1 } }"#);
    let draft = AggregateState {
        stages: edited.stages,
        ..draft
    };
    let next = plan_next(&draft, Some(&ns()), true);
    let keys = next.suggested.expect("a suggestion").keys;
    assert_eq!(keys[0].0, "status");
}

#[test]
fn the_command_uses_the_plan_s_collation() {
    let options = PipelineOptions {
        collation: Some(collation("fr")),
        ..PipelineOptions::default()
    };
    let draft = explained("collscan", options);
    // The draft's own options change afterwards; the plan keeps the collation it was made with.
    let draft = AggregateState {
        options: PipelineOptions::default(),
        ..draft
    };
    let command = plan_next(&draft, Some(&ns()), true)
        .command
        .expect("a command");
    assert!(
        command.contains(r#"collation: { locale: "fr", strength: 2 }"#),
        "{command}"
    );
}

#[test]
fn a_running_explain_or_a_blocked_form_turns_try_off_with_its_reason() {
    let draft = explained("collscan", PipelineOptions::default());
    let blocked =
        draft
            .clone()
            .options_edited(crate::viewmodel::aggregate_options::OptionsEdit::MaxTime(
                "0".into(),
            ));
    assert_eq!(
        plan_next(&blocked, Some(&ns()), true).blocked,
        Some("aggregate.options.blocked")
    );
    let (running, _, _) = draft
        .explain
        .clone()
        .begun(pipeline(), &PipelineOptions::default());
    let busy = AggregateState {
        explain: running,
        ..draft
    };
    assert_eq!(
        plan_next(&busy, Some(&ns()), true).blocked,
        Some("aggregate.explain.disabled.running")
    );
}

/// A draft whose explain failed after `options` were sent with it.
fn failed_with(options: PipelineOptions) -> AggregateState {
    let draft = AggregateState::default();
    let (ui, request, _) = draft.explain.clone().begun(pipeline(), &options);
    let ui = ui.arrived(request, Err(curlybrace_core::error::CoreError::Unreachable));
    AggregateState {
        explain: ui,
        options,
        ..draft
    }
}

#[test]
fn a_failed_explain_keeps_clear_hint_reachable_when_a_hint_is_set() {
    let hinted = PipelineOptions {
        hint: Some(HintOption::Name("nope_1".into())),
        ..PipelineOptions::default()
    };
    let next = plan_next(&failed_with(hinted), Some(&ns()), true);
    assert!(next.try_index, "a bad hint must be clearable");
    assert_eq!(next.current_hint.as_deref(), Some("nope_1"));
    assert_eq!(next.suggested, None);
    assert_eq!(next.blocked, None);
}

#[test]
fn a_failed_explain_without_a_hint_offers_nothing() {
    let next = plan_next(&failed_with(PipelineOptions::default()), Some(&ns()), true);
    assert!(!next.try_index);
    assert_eq!(next.current_hint, None);
}

#[test]
fn a_lost_connection_turns_try_off_with_its_reason() {
    let draft = explained("collscan", PipelineOptions::default());
    assert_eq!(
        plan_next(&draft, Some(&ns()), false).blocked,
        Some("aggregate.explain.disabled.no_connection")
    );
    assert_eq!(plan_next(&draft, Some(&ns()), true).blocked, None);
}

#[test]
fn a_running_explain_gives_its_reason_before_a_lost_connection() {
    let draft = explained("collscan", PipelineOptions::default());
    let (running, _, _) = draft
        .explain
        .clone()
        .begun(pipeline(), &PipelineOptions::default());
    let busy = AggregateState {
        explain: running,
        ..draft
    };
    assert_eq!(
        plan_next(&busy, Some(&ns()), false).blocked,
        Some("aggregate.explain.disabled.running")
    );
}

#[test]
fn the_current_hint_is_shown_by_name_or_keys() {
    let by_keys = PipelineOptions {
        hint: Some(HintOption::Keys(doc! { "status": 1_i32, "total": -1_i32 })),
        ..PipelineOptions::default()
    };
    let next = plan_next(&explained("collscan", by_keys), Some(&ns()), true);
    assert_eq!(
        next.current_hint,
        Some(crate::viewmodel::aggregate_options::keys_text(
            &doc! { "status": 1_i32, "total": -1_i32 }
        ))
    );
}
