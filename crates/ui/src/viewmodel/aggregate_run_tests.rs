use super::*;
use curlybrace_core::pipeline::Stage;

fn stage(text: &str, enabled: bool) -> Stage {
    Stage {
        text: text.to_string(),
        enabled,
    }
}

fn draft(texts: &[(&str, bool)]) -> AggregateState {
    AggregateState {
        stages: texts.iter().map(|(t, e)| stage(t, *e)).collect(),
        keys: (0..texts.len() as u64).collect(),
        ..AggregateState::default()
    }
}

const MATCH: &str = r#"{ "$match": {} }"#;
const OUT: &str = r#"{ "$out": "dest" }"#;

fn env() -> RunEnv {
    RunEnv {
        connected: true,
        has_collection: true,
        read_only: false,
    }
}

fn dialog() -> RunDialog {
    RunDialog {
        target: RunTarget {
            db: None,
            collection: "dest".into(),
        },
        stage: "$out".into(),
        source: Namespace::new("app", "people"),
        typed: String::new(),
        target_count: None,
    }
}

#[test]
fn run_is_hidden_unless_the_last_enabled_stage_writes() {
    assert_eq!(
        AggregateState::default().can_run(env()),
        RunAvailability::Hidden
    );
    assert_eq!(
        draft(&[(MATCH, true)]).can_run(env()),
        RunAvailability::Hidden
    );
    assert_eq!(
        draft(&[(OUT, true), (MATCH, true)]).can_run(env()),
        RunAvailability::Hidden
    );
    // A disabled trailing write stage does not count; the last enabled stage decides.
    assert_eq!(
        draft(&[(MATCH, true), (OUT, false)]).can_run(env()),
        RunAvailability::Hidden
    );
    assert_eq!(
        draft(&[(MATCH, true), ("{ nope", true)]).can_run(env()),
        RunAvailability::Hidden
    );
}

#[test]
fn run_is_ready_for_a_pipeline_ending_in_out_or_merge() {
    assert_eq!(
        draft(&[(MATCH, true), (OUT, true)]).can_run(env()),
        RunAvailability::Ready
    );
    assert_eq!(
        draft(&[(r#"{ "$merge": { "into": "dest" } }"#, true)]).can_run(env()),
        RunAvailability::Ready
    );
    assert_eq!(
        draft(&[(OUT, false), (MATCH, true), (OUT, true)]).can_run(env()),
        RunAvailability::Ready
    );
}

#[test]
fn run_is_disabled_with_the_reason() {
    let d = draft(&[(MATCH, true), (OUT, true)]);
    let with = |read_only, connected, has_collection| {
        d.can_run(RunEnv {
            connected,
            has_collection,
            read_only,
        })
    };
    assert_eq!(
        with(true, true, true),
        RunAvailability::Disabled(RunDisabled::ReadOnly)
    );
    assert_eq!(
        with(false, false, true),
        RunAvailability::Disabled(RunDisabled::NoConnection)
    );
    assert_eq!(
        with(false, true, false),
        RunAvailability::Disabled(RunDisabled::NoCollection)
    );
}

#[test]
fn run_is_disabled_for_an_unrunnable_pipeline() {
    let bad_target = draft(&[(MATCH, true), (r#"{ "$out": "" }"#, true)]);
    assert!(matches!(
        bad_target.can_run(env()),
        RunAvailability::Disabled(RunDisabled::Invalid(PipelineError::TargetUnresolved { .. }))
    ));
    let unparsable = draft(&[("{ nope", true), (OUT, true)]);
    assert!(matches!(
        unparsable.can_run(env()),
        RunAvailability::Disabled(RunDisabled::Invalid(PipelineError::NotAStage { index: 0 }))
    ));
    let second_write = draft(&[(OUT, true), (OUT, true)]);
    assert!(matches!(
        second_write.can_run(env()),
        RunAvailability::Disabled(RunDisabled::Invalid(
            PipelineError::WriteStageNotLast { .. }
        ))
    ));
}

#[test]
fn run_is_disabled_while_a_run_is_in_flight_and_for_unapplied_text() {
    let d = draft(&[(MATCH, true), (OUT, true)]);
    let busy = AggregateState {
        run: RunUi::default().started(1),
        ..d.clone()
    };
    assert_eq!(
        busy.can_run(env()),
        RunAvailability::Disabled(RunDisabled::Busy)
    );
    let text = d.to_text();
    assert_eq!(text.clone().can_run(env()), RunAvailability::Ready);
    let edited = text.with_text(r#"[{ "$out": "other" }]"#);
    assert_eq!(
        edited.can_run(env()),
        RunAvailability::Disabled(RunDisabled::TextNotApplied)
    );
}

#[test]
fn the_typed_text_must_equal_the_target_collection() {
    let typed = |t: &str| RunDialog {
        typed: t.into(),
        ..dialog()
    };
    assert!(typed("dest").is_confirmed());
    assert!(typed("  dest \n").is_confirmed());
    assert!(!typed("Dest").is_confirmed());
    assert!(!typed("des").is_confirmed());
    assert!(!typed("").is_confirmed());
    assert!(!typed("de st").is_confirmed());
}

#[test]
fn the_target_namespace_falls_back_to_the_source_database() {
    assert_eq!(dialog().target_ns(), Namespace::new("app", "dest"));
    let other = RunDialog {
        target: RunTarget {
            db: Some("warehouse".into()),
            collection: "dest".into(),
        },
        ..dialog()
    };
    assert_eq!(other.target_ns(), Namespace::new("warehouse", "dest"));
}

#[test]
fn the_count_fills_the_dialog_only_for_its_target() {
    let ui = RunUi::default().opened(dialog());
    let wrong = ui
        .clone()
        .count_arrived(&Namespace::new("app", "people"), 9);
    assert_eq!(wrong.dialog.unwrap().target_count, None);
    let right = ui.count_arrived(&Namespace::new("app", "dest"), 7);
    assert_eq!(right.dialog.unwrap().target_count, Some(7));
}

#[test]
fn only_the_answer_of_the_run_in_flight_counts() {
    let done = RunStatus::Done {
        target: Namespace::new("app", "dest"),
        count: Some(1),
    };
    let ui = RunUi::default().started(3);
    assert_eq!(
        ui.clone().answered(2, done.clone()).status,
        RunStatus::Running
    );
    let finished = ui.answered(3, done.clone());
    assert_eq!(finished.status, done);
    assert_eq!(finished.answered(3, RunStatus::Unknown).status, done);
}

#[test]
fn a_command_failure_is_unknown_when_the_write_may_have_happened() {
    let ui = RunUi::default().started(1);
    assert_eq!(
        ui.clone()
            .command_failed(CoreError::WriteOutcomeUnknown)
            .status,
        RunStatus::Unknown
    );
    assert_eq!(
        ui.clone().command_failed(CoreError::Unreachable).status,
        RunStatus::Unknown
    );
    assert_eq!(
        ui.command_failed(CoreError::Database("x".into())).status,
        RunStatus::Failed(CoreError::Database("x".into()))
    );
}

#[test]
fn changing_the_pipeline_or_the_collection_closes_the_dialog() {
    let open = AggregateState {
        run: RunUi::default().opened(dialog()),
        ..draft(&[(MATCH, true), (OUT, true)])
    };
    let edited = open.clone().edit(0, r#"{ "$match": { "a": 1 } }"#);
    assert!(edited.run.dialog.is_none());
    assert!(open.clone().toggle(0).run.dialog.is_none());
    assert!(open.clone().remove(0).run.dialog.is_none());
    assert!(open.clone().add_stage("$sort").run.dialog.is_none());
    assert!(open.clone().move_up(1).run.dialog.is_none());
    assert!(open.clone().on_collection_changed().run.dialog.is_none());
    assert!(open.clone().on_connection_reset().run.dialog.is_none());
    assert!(open.with_run_dialog_closed().run.dialog.is_none());
}

#[test]
fn a_collection_change_forgets_a_finished_run_but_not_one_in_flight_or_unknown() {
    let done = RunStatus::Done {
        target: Namespace::new("app", "dest"),
        count: None,
    };
    let status = |s: RunStatus| {
        RunUi {
            status: s,
            ..RunUi::default()
        }
        .collection_changed()
        .status
    };
    assert_eq!(status(done), RunStatus::Idle);
    assert_eq!(
        status(RunStatus::Failed(CoreError::ReadOnly)),
        RunStatus::Idle
    );
    assert_eq!(status(RunStatus::Running), RunStatus::Running);
    assert_eq!(status(RunStatus::Unknown), RunStatus::Unknown);
}

#[test]
fn a_lost_connection_makes_a_run_in_flight_unknown() {
    let ui = RunUi::default().started(5).connection_reset();
    assert_eq!(ui.status, RunStatus::Unknown);
    assert_eq!(ui.run, 5);
}
