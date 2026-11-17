use super::*;
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_counts_plan::NotCounted;
use crate::viewmodel::aggregate_counts_plan::count_plan;
use curlybrace_core::pipeline::PipelineOptions;
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;

fn draft() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$group")
        .add_stage("$sort")
}

fn started(d: &AggregateState) -> (CountsUi, CountSend) {
    let plan = count_plan(d, &d.stages).expect("a plan");
    let (ui, send) =
        CountsUi::default().begun(plan, &d.stages, &d.keys, &PipelineOptions::default());
    (ui, send.expect("the first count is sent"))
}

#[test]
fn the_answers_are_told_apart_into_exact_at_least_timed_out_and_failed() {
    assert_eq!(StageCount::of(Ok(0)), StageCount::Exact(0));
    assert_eq!(StageCount::of(Ok(7)), StageCount::Exact(7));
    assert_eq!(StageCount::of(Ok(CAP - 1)), StageCount::Exact(CAP - 1));
    assert_eq!(StageCount::of(Ok(CAP)), StageCount::AtLeast(CAP));
    assert_eq!(
        StageCount::of(Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into()))),
        StageCount::TimedOut
    );
    assert_eq!(
        StageCount::of(Err(CoreError::Unreachable)),
        StageCount::Failed(CoreError::Unreachable)
    );
}

#[test]
fn the_first_stage_is_sent_and_the_others_wait_for_their_turn() {
    let d = draft();
    let (ui, send) = started(&d);
    assert_eq!(send.stage_index, 0);
    assert!(ui.is_running());
    assert!(ui.awaits(send.request));
    assert_eq!(ui.progress_of(0), Some(Progress::Counting));
    assert_eq!(ui.progress_of(1), Some(Progress::Waiting));
    assert_eq!(ui.progress_of(2), Some(Progress::Waiting));
}

#[test]
fn each_answer_sends_the_next_stage_in_order_and_the_last_ends_the_sequence() {
    let d = draft();
    let (ui, first) = started(&d);
    let (ui, second) = ui.answered(first.request, 0, Ok(10));
    let second = second.expect("stage 1 follows");
    assert_eq!(second.stage_index, 1);
    assert_ne!(second.request, first.request);
    let (ui, third) = ui.answered(second.request, 1, Ok(4));
    let third = third.expect("stage 2 follows");
    assert_eq!(third.stage_index, 2);
    let (ui, none) = ui.answered(third.request, 2, Ok(CAP));
    assert_eq!(none, None);
    assert!(!ui.is_running());
    assert_eq!(ui.result_of(d.keys[0]), Some(&StageCount::Exact(10)));
    assert_eq!(ui.result_of(d.keys[1]), Some(&StageCount::Exact(4)));
    assert_eq!(ui.result_of(d.keys[2]), Some(&StageCount::AtLeast(CAP)));
}

#[test]
fn a_failure_on_one_stage_does_not_stop_the_next() {
    let d = draft();
    let (ui, first) = started(&d);
    let (ui, second) = ui.answered(first.request, 0, Err(CoreError::Database("boom".into())));
    assert!(second.is_some());
    assert_eq!(
        ui.result_of(d.keys[0]),
        Some(&StageCount::Failed(CoreError::Database("boom".into())))
    );
}

#[test]
fn an_answer_for_another_request_or_stage_changes_nothing() {
    let d = draft();
    let (ui, first) = started(&d);
    let (same, none) = ui.clone().answered(first.request + 1, 0, Ok(1));
    assert_eq!((same.clone(), none), (ui.clone(), None));
    let (same, none) = ui.clone().answered(first.request, 1, Ok(1));
    assert_eq!((same, none), (ui, None));
}

#[test]
fn cancelling_keeps_the_results_and_ignores_the_late_answer() {
    let d = draft();
    let (ui, first) = started(&d);
    let (ui, second) = ui.answered(first.request, 0, Ok(3));
    let second = second.expect("sent");
    let cancelled = ui.cancelled();
    assert!(!cancelled.is_running());
    assert_eq!(cancelled.result_of(d.keys[0]), Some(&StageCount::Exact(3)));
    let (late, none) = cancelled.clone().answered(second.request, 1, Ok(9));
    assert_eq!((late, none), (cancelled, None));
}

#[test]
fn a_second_start_while_running_is_ignored() {
    let d = draft();
    let (ui, first) = started(&d);
    let plan = count_plan(&d, &d.stages).expect("a plan");
    let (again, send) = ui
        .clone()
        .begun(plan, &d.stages, &d.keys, &PipelineOptions::default());
    assert_eq!(send, None);
    assert_eq!(again, ui);
    assert!(again.awaits(first.request));
}

#[test]
fn stages_that_cannot_be_counted_say_so_at_once_and_are_never_sent() {
    let d = draft().edit(1, "{ \"$out\": \"x\" }");
    let (ui, first) = started(&d);
    assert_eq!(
        ui.result_of(d.keys[1]),
        Some(&StageCount::NotCounted(NotCounted::WriteStage))
    );
    assert_eq!(
        ui.result_of(d.keys[2]),
        Some(&StageCount::NotCounted(NotCounted::WriteStage))
    );
    let (ui, none) = ui.answered(first.request, 0, Ok(5));
    assert_eq!(none, None, "nothing else to send");
    assert!(!ui.is_running());
}

#[test]
fn a_new_sequence_drops_the_old_results_and_never_reuses_a_request_number() {
    let d = draft();
    let (ui, first) = started(&d);
    let ui = ui.cancelled();
    let plan = count_plan(&d, &d.stages).expect("a plan");
    let (ui, held) = ui.begun(plan, &d.stages, &d.keys, &PipelineOptions::default());
    assert_eq!(held, None, "the cancelled count is still out");
    let (ui, again) = ui.released(first.request);
    let again = again.expect("sent");
    assert!(again.request > first.request);
    assert_eq!(ui.result_of(d.keys[0]), None);
}

#[test]
fn results_are_out_of_date_once_the_stages_differ_from_the_counted_ones() {
    let d = draft();
    let (ui, first) = started(&d);
    assert!(!ui.is_out_of_date(&d.stages, &PipelineOptions::default()));
    let edited = d.clone().edit(0, "{ \"$match\": { \"a\": 1 } }");
    assert!(ui.is_out_of_date(&edited.stages, &PipelineOptions::default()));
    assert!(ui.is_out_of_date(&d.clone().toggle(2).stages, &PipelineOptions::default()));
    assert!(
        !CountsUi::default().is_out_of_date(&d.stages, &PipelineOptions::default()),
        "nothing counted"
    );
    let _ = first;
}

#[test]
fn a_reset_forgets_everything_and_keeps_the_numbering() {
    let d = draft();
    let (ui, first) = started(&d);
    let (ui, _) = ui.answered(first.request, 0, Ok(3));
    let reset = ui.reset();
    assert!(!reset.is_running());
    assert_eq!(reset.result_of(d.keys[0]), None);
    assert!(!reset.is_out_of_date(&[], &PipelineOptions::default()));
}

#[test]
fn giving_up_ends_only_the_awaited_request() {
    let d = draft();
    let (ui, first) = started(&d);
    assert!(ui.clone().given_up(first.request + 5).is_running());
    assert!(!ui.given_up(first.request).is_running());
}

#[test]
fn the_debug_text_names_no_pipeline_and_no_number() {
    let d = draft();
    let (ui, first) = started(&d);
    let (ui, _) = ui.answered(first.request, 0, Ok(123_456));
    let text = format!("{ui:?}");
    assert!(
        !text.contains("123456") && !text.contains("$match"),
        "{text}"
    );
}

/// A sequence cancelled with its first count still out, and a new Start held back behind it.
fn held_behind_the_cancelled(d: &AggregateState) -> (CountsUi, CountSend) {
    let (ui, first) = started(d);
    let plan = count_plan(d, &d.stages).expect("a plan");
    let (ui, send) = ui
        .cancelled()
        .begun(plan, &d.stages, &d.keys, &PipelineOptions::default());
    assert_eq!(send, None, "nothing goes out while the old count is out");
    (ui, first)
}

#[test]
fn a_start_after_a_cancel_waits_for_the_count_still_out() {
    let d = draft();
    let (ui, first) = held_behind_the_cancelled(&d);
    assert!(ui.is_running(), "Cancel is offered for the held start");
    assert!(!ui.awaits(first.request));
    assert_eq!(ui.progress_of(0), Some(Progress::Waiting));
    assert_eq!(ui.progress_of(2), Some(Progress::Waiting));
}

#[test]
fn the_answer_of_the_old_count_releases_the_held_start_and_is_never_a_result() {
    let d = draft();
    let (ui, first) = held_behind_the_cancelled(&d);
    let (ui, none) = ui.clone().answered(first.request, 0, Ok(99));
    assert_eq!(none, None, "`answered` is for the count awaited only");
    assert_eq!(ui.result_of(d.keys[0]), None);
    let (ui, send) = ui.released(first.request);
    let send = send.expect("stage 0 of the new sequence goes out");
    assert_eq!(send.stage_index, 0);
    assert!(send.request > first.request);
    assert!(ui.awaits(send.request));
    assert_eq!(ui.result_of(d.keys[0]), None);
}

#[test]
fn another_request_releases_nothing() {
    let d = draft();
    let (ui, first) = held_behind_the_cancelled(&d);
    let (same, none) = ui.clone().released(first.request + 7);
    assert_eq!((same, none), (ui, None));
}

#[test]
fn a_release_without_a_held_start_only_frees_the_slot() {
    let d = draft();
    let (ui, first) = started(&d);
    let ui = ui.cancelled();
    let (ui, none) = ui.released(first.request);
    assert_eq!(none, None);
    let plan = count_plan(&d, &d.stages).expect("a plan");
    assert!(
        ui.begun(plan, &d.stages, &d.keys, &PipelineOptions::default())
            .1
            .is_some(),
        "free again"
    );
}

#[test]
fn cancelling_a_held_start_keeps_the_old_count_out() {
    let d = draft();
    let (ui, first) = held_behind_the_cancelled(&d);
    let ui = ui.cancelled();
    assert!(!ui.is_running());
    let plan = count_plan(&d, &d.stages).expect("a plan");
    let (ui, send) = ui.begun(plan, &d.stages, &d.keys, &PipelineOptions::default());
    assert_eq!(send, None, "still held behind the old count");
    assert_eq!(ui.released(first.request).1.map(|s| s.stage_index), Some(0));
}

#[test]
fn giving_up_the_old_count_frees_the_slot_and_ends_the_held_start() {
    let d = draft();
    let (ui, first) = held_behind_the_cancelled(&d);
    let ui = ui.given_up(first.request);
    assert!(!ui.is_running());
    let plan = count_plan(&d, &d.stages).expect("a plan");
    assert!(
        ui.begun(plan, &d.stages, &d.keys, &PipelineOptions::default())
            .1
            .is_some()
    );
}

#[test]
fn a_reset_keeps_the_count_still_out_and_drops_the_held_start() {
    let d = draft();
    let (ui, first) = held_behind_the_cancelled(&d);
    let ui = ui.reset();
    assert!(!ui.is_running());
    assert!(ui.lingers(first.request));
    let plan = count_plan(&d, &d.stages).expect("a plan");
    let (ui, none) = ui.begun(plan, &d.stages, &d.keys, &PipelineOptions::default());
    assert_eq!(none, None, "a new start waits for the old count");
    assert!(ui.released(first.request).1.is_some());
}

#[test]
fn a_reset_while_counting_leaves_that_count_out() {
    let d = draft();
    let (ui, first) = started(&d);
    assert!(ui.reset().lingers(first.request));
}

#[test]
fn a_lost_connection_forgets_the_count_still_out() {
    let d = draft();
    let (ui, _) = held_behind_the_cancelled(&d);
    let ui = ui.connection_lost();
    assert!(!ui.is_running());
    let plan = count_plan(&d, &d.stages).expect("a plan");
    assert!(
        ui.begun(plan, &d.stages, &d.keys, &PipelineOptions::default())
            .1
            .is_some()
    );
}

#[test]
fn request_numbers_are_unique_across_fresh_counts() {
    let d = draft();
    let (_, first) = started(&d);
    let (_, second) = started(&d);
    assert_ne!(first.request, second.request);
}
