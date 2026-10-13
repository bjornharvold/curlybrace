//! The check queue: one `Check` in flight, a fresh probe id per check, stale answers dropped.
use super::pairs::{all_unchecked, multi_state};
use super::*;
use crate::viewmodel::compare::{EntryCheck, PairOverrides};

fn check_sent(cmds: &[Command]) -> (u32, String) {
    match cmds {
        [Command::Compare(CompareCommand::Check { session: 1, pair })] => {
            (pair.id, pair.source.collection.clone())
        }
        other => panic!("expected exactly one Check, got {other:?}"),
    }
}

fn checking(state: &AppState) -> usize {
    state
        .compare
        .checks
        .entries
        .values()
        .filter(|c| matches!(c, EntryCheck::Checking(_)))
        .count()
}

#[test]
fn run_check_sends_one_check_at_a_time_with_distinct_probe_ids() {
    let (state, cmds) = act(
        multi_state(&["alpha", "beta", "gamma"]),
        CompareAction::RunCheck,
    );
    assert_eq!(check_sent(&cmds), (1, "alpha".to_string()));
    assert_eq!(checking(&state), 1);
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty(), "no second check while one is in flight");
    let (state, cmds) = update(state, checked_as(1, indexed(5)));
    assert_eq!(check_sent(&cmds), (2, "beta".to_string()));
    assert_eq!(checking(&state), 1);
    let (state, cmds) = update(state, checked_as(2, indexed(5)));
    assert_eq!(check_sent(&cmds), (3, "gamma".to_string()));
    let (state, cmds) = update(state, checked_as(3, indexed(5)));
    assert!(cmds.is_empty() && checking(&state) == 0);
    assert!(!state.compare.check_in_flight());
    assert!(state.compare.can_start());
}

#[test]
fn checked_for_an_older_probe_is_dropped_and_does_not_resume_the_queue() {
    let (state, _) = act(multi_state(&["alpha", "beta"]), CompareAction::RunCheck);
    let (state, _) = act(state, CompareAction::SetKey("sku".into()));
    assert!(all_unchecked(&state));
    let (state, cmds) = update(state, checked_as(1, indexed(5)));
    assert!(cmds.is_empty(), "the queue is not resumed");
    assert!(all_unchecked(&state) && !state.compare.check_in_flight());
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert_eq!(check_sent(&cmds).0, 2, "a new queue, a new probe");
    let (state, cmds) = update(state, checked_as(1, indexed(5)));
    assert!(cmds.is_empty(), "the old probe stays dead");
    assert_eq!(checking(&state), 1);
}

#[test]
fn an_answer_under_an_unknown_probe_changes_nothing() {
    let (state, _) = act(multi_state(&["alpha", "beta"]), CompareAction::RunCheck);
    let (state, cmds) = update(state, checked_as(77, indexed(5)));
    assert!(cmds.is_empty() && checking(&state) == 1);
}

#[test]
fn a_failed_check_continues_with_the_next_entry() {
    let (state, _) = act(multi_state(&["alpha", "beta"]), CompareAction::RunCheck);
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_check",
        error: CoreError::NotFound,
    });
    let (state, cmds) = update(state, failed);
    assert_eq!(
        state.compare.checks.entries.get(&0),
        Some(&EntryCheck::Failed)
    );
    assert_eq!(check_sent(&cmds), (2, "beta".to_string()));
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_check",
            ..
        })
    ));
    let (state, _) = update(state, checked_as(2, indexed(5)));
    assert!(state.compare.checks.verdict(1).is_some());
    assert!(!state.compare.check_in_flight());
}

#[test]
fn invalid_entries_are_not_queued() {
    let state = multi_state(&["alpha", "beta"]);
    let broken = PairOverrides {
        key_text: Some("a,,b".into()),
        ..PairOverrides::default()
    };
    let (state, _) = act(state, CompareAction::SetPairOverrides(0, broken));
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert_eq!(check_sent(&cmds), (1, "beta".to_string()));
    let (state, cmds) = update(state, checked_as(1, indexed(5)));
    assert!(cmds.is_empty(), "nothing else is queued");
    assert_eq!(
        state.compare.checks.entries.get(&0),
        None,
        "the invalid entry never entered the queue"
    );
}

#[test]
fn nothing_valid_to_check_sends_nothing() {
    let state = multi_state(&["alpha"]);
    let broken = PairOverrides {
        key_text: Some("a,,b".into()),
        ..PairOverrides::default()
    };
    let (state, _) = act(state, CompareAction::SetPairOverrides(0, broken));
    let (_, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty());
}

#[test]
fn excluded_entries_are_not_queued() {
    let (state, _) = act(
        multi_state(&["alpha", "beta"]),
        CompareAction::IncludePair(0, false),
    );
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert_eq!(check_sent(&cmds), (1, "beta".to_string()));
    let (state, _) = update(state, checked_as(1, indexed(5)));
    assert!(
        state.compare.can_start(),
        "only the included entry has to pass"
    );
}
