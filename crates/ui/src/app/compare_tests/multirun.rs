//! Runs over several entries: fresh pair ids, one `Start`, per-pair outcomes, cancelling,
//! discarding, re-running and the late events that must not change anything.
use super::pairs::{multi_state, status};
use super::*;
use crate::viewmodel::compare::{EntryId, PairStatus};
use curlybrace_core::compare::protocol::PairSpec;

pub(super) const NAMES: [&str; 3] = ["alpha", "beta", "gamma"];

/// Checks every included entry in turn; probes are 1.. in order, so the next pair id is
/// `count + 1`.
pub(super) fn checked_all(state: AppState, count: u32) -> AppState {
    let first = state.compare.next_pair;
    let (mut state, cmds) = act(state, CompareAction::RunCheck);
    assert_eq!(cmds.len(), 1);
    for probe in first..first + count {
        state = update(state, checked_as(probe, indexed(5))).0;
    }
    assert!(state.compare.can_start(), "{:?}", state.compare.checks);
    state
}

/// Three entries, all checked; the first run's pairs are 4, 5 and 6.
pub(super) fn ready_three() -> AppState {
    checked_all(multi_state(&NAMES), 3)
}

pub(super) fn started_specs(cmds: &[Command]) -> Vec<PairSpec> {
    match cmds.last() {
        Some(Command::Compare(CompareCommand::Start { pairs, .. })) => pairs.clone(),
        other => panic!("expected a Start, got {other:?}"),
    }
}

pub(super) fn running_three() -> AppState {
    act(ready_three(), CompareAction::Start).0
}

pub(super) fn done(run: u64, pair: u32, outcome: PairOutcome) -> Input {
    ev(CompareEvent::PairDone {
        session: 1,
        run,
        pair,
        outcome,
    })
}

pub(super) fn completed(run: u64, pair: u32) -> Input {
    let summary = Summary {
        different: 1,
        ..Summary::default()
    };
    done(run, pair, PairOutcome::Completed(summary))
}

pub(super) fn failed_with(run: u64, pair: u32, error: CoreError) -> Input {
    done(run, pair, PairOutcome::Failed(error))
}

pub(super) fn run_done_of(run: u64) -> Input {
    ev(CompareEvent::RunDone { session: 1, run })
}

pub(super) fn rows_for(pair: u32, n: i32) -> Input {
    ev(CompareEvent::Rows {
        session: 1,
        pair,
        category: Category::Different,
        offset: 0,
        total: 1,
        rows: (0..n)
            .map(|i| RowView {
                key: KeyTuple(vec![Bson::Int32(i)]),
                has_source: true,
                has_target: true,
            })
            .collect(),
    })
}

pub(super) fn feed(state: AppState, inputs: impl IntoIterator<Item = Input>) -> AppState {
    inputs.into_iter().fold(state, |s, i| update(s, i).0)
}

/// All three pairs of run 1 completed and the run over.
pub(super) fn finished_three() -> AppState {
    feed(
        running_three(),
        [
            completed(1, 4),
            completed(1, 5),
            completed(1, 6),
            run_done_of(1),
        ],
    )
}

pub(super) fn pair_of(state: &AppState, entry: EntryId) -> u32 {
    state.compare.overview.row_for(entry).unwrap().pair
}

#[test]
fn start_assigns_fresh_pair_ids_and_sends_one_start_in_list_order() {
    let (state, cmds) = act(ready_three(), CompareAction::Start);
    assert_eq!(cmds.len(), 1, "one Start, no Discard");
    let specs = started_specs(&cmds);
    let ids: Vec<u32> = specs.iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![4, 5, 6]);
    let names: Vec<&str> = specs.iter().map(|s| s.source.collection.as_str()).collect();
    assert_eq!(names, NAMES.to_vec());
    assert_eq!(state.compare.next_pair, 7);
    assert!(state.compare.is_running());
    let rows: Vec<(EntryId, u32)> = state
        .compare
        .overview
        .rows
        .iter()
        .map(|r| (r.entry, r.pair))
        .collect();
    assert_eq!(rows, vec![(0, 4), (1, 5), (2, 6)]);
}

#[test]
fn second_run_never_reuses_a_pair_id() {
    let state = finished_three();
    let (state, cmds) = act(state, CompareAction::Start);
    let discarded: Vec<u32> = cmds
        .iter()
        .filter_map(|c| match c {
            Command::Compare(CompareCommand::Discard { pair, .. }) => Some(*pair),
            _ => None,
        })
        .collect();
    assert_eq!(discarded, vec![4, 5, 6], "the superseded pairs go first");
    let ids: Vec<u32> = started_specs(&cmds).iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![7, 8, 9]);
    assert!(state.compare.results.is_empty());
}

#[test]
fn start_is_refused_with_nothing_included() {
    let mut state = multi_state(&NAMES);
    for entry in 0..3 {
        state = act(state, CompareAction::IncludePair(entry, false)).0;
    }
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty(), "nothing included, nothing to check");
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty() && !state.compare.is_running());
    let (_, cmds) = act(state, CompareAction::StartPassing);
    assert!(cmds.is_empty());
}

#[test]
fn start_runs_only_the_included_entries() {
    let state = multi_state(&NAMES);
    let (state, _) = act(state, CompareAction::IncludePair(1, false));
    let state = checked_all(state, 2);
    let (_, cmds) = act(state, CompareAction::Start);
    let names: Vec<String> = started_specs(&cmds)
        .iter()
        .map(|s| s.source.collection.clone())
        .collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
}

#[test]
fn start_is_refused_while_checking_or_running() {
    let (checking, _) = act(multi_state(&NAMES), CompareAction::RunCheck);
    let (_, cmds) = act(checking, CompareAction::Start);
    assert!(cmds.is_empty());
    let (_, cmds) = act(running_three(), CompareAction::Start);
    assert!(cmds.is_empty(), "one run slot");
    let (_, cmds) = act(running_three(), CompareAction::StartPassing);
    assert!(cmds.is_empty());
    let (_, cmds) = act(running_three(), CompareAction::RunCheck);
    assert!(cmds.is_empty(), "no check during a run");
}

#[test]
fn start_passing_skips_refused_entries() {
    let state = multi_state(&NAMES);
    let (state, _) = act(state, CompareAction::RunCheck);
    let blocked = Preflight {
        unindexed_too_large: true,
        source_index: IndexStatus::NotIndexed,
        ..indexed(60_000)
    };
    let state = feed(
        state,
        [
            checked_as(1, indexed(5)),
            checked_as(2, blocked),
            checked_as(3, indexed(5)),
        ],
    );
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(cmds.is_empty(), "Run is refused while an entry is refused");
    let (state, cmds) = act(state, CompareAction::StartPassing);
    let names: Vec<String> = started_specs(&cmds)
        .iter()
        .map(|s| s.source.collection.clone())
        .collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
    assert!(state.compare.overview.row_for(1).is_none());
}

#[test]
fn a_failing_pair_between_two_good_ones_does_not_stop_them() {
    let state = running_three();
    let (state, first) = update(state, completed(1, 4));
    let (state, second) = update(state, failed_with(1, 5, CoreError::CompareKeyMissing));
    let (state, third) = update(state, completed(1, 6));
    let state = update(state, run_done_of(1)).0;
    assert!(matches!(status(&state, 0), PairStatus::Completed(_)));
    assert!(matches!(
        status(&state, 1),
        PairStatus::Failed(CoreError::CompareKeyMissing)
    ));
    assert!(matches!(status(&state, 2), PairStatus::Completed(_)));
    assert!(!state.compare.is_running());
    let rows_requested = |cmds: &[Command], pair: u32| {
        cmds.iter().any(
            |c| matches!(c, Command::Compare(CompareCommand::Rows { pair: p, .. }) if *p == pair),
        )
    };
    assert!(rows_requested(&first, 4) && rows_requested(&third, 6));
    assert!(second.is_empty(), "a failed pair has no rows to ask for");
    let held: Vec<u32> = state.compare.results.keys().copied().collect();
    assert_eq!(held, vec![4, 6]);
}

#[test]
fn banner_is_raised_once_per_run_for_failed_pairs() {
    let state = running_three();
    let (state, _) = update(state, failed_with(1, 4, CoreError::CompareKeyMissing));
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_start",
            ..
        })
    ));
    let state = AppState {
        banner: None,
        ..state
    };
    let state = update(state, failed_with(1, 5, CoreError::CompareDuplicateKey)).0;
    assert!(
        state.banner.is_none(),
        "the second failure of the run is quiet"
    );
    let state = feed(state, [completed(1, 6), run_done_of(1)]);
    let (state, _) = act(state, CompareAction::Start);
    let state = update(state, failed_with(2, 7, CoreError::CompareKeyMissing)).0;
    assert!(state.banner.is_some(), "a new run raises it again");
}

#[test]
fn cancel_is_sent_once_and_rows_end_only_by_runtime_events() {
    let (state, cmds) = act(running_three(), CompareAction::Cancel);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Cancel {
            session: 1,
            run: 1
        })]
    ));
    let (state, cmds) = act(state, CompareAction::Cancel);
    assert!(cmds.is_empty(), "a second Cancel sends nothing");
    for entry in 0..3 {
        assert_eq!(
            status(&state, entry),
            PairStatus::Pending,
            "no local guessing"
        );
    }
    let cancelled = done(1, 4, PairOutcome::Cancelled(Summary::default()));
    let (state, _) = update(state, cancelled);
    assert!(matches!(status(&state, 0), PairStatus::Cancelled(_)));
    assert_eq!(
        status(&state, 1),
        PairStatus::Pending,
        "still waiting for the runtime"
    );
    let state = update(state, run_done_of(1)).0;
    assert!(!state.compare.is_running());
    for entry in 0..3 {
        assert!(matches!(status(&state, entry), PairStatus::Cancelled(_)));
    }
    assert!(state.compare.results.is_empty());
}

#[test]
fn progress_updates_only_the_matching_active_row() {
    let progress = |run, pair| {
        ev(CompareEvent::Progress {
            session: 1,
            run,
            pair,
            scanned_source: 5,
            scanned_target: 6,
            differences: 1,
        })
    };
    let state = feed(
        running_three(),
        [progress(1, 5), progress(1, 99), progress(9, 4)],
    );
    assert!(matches!(status(&state, 1), PairStatus::Running { .. }));
    assert_eq!(status(&state, 0), PairStatus::Pending);
    assert_eq!(status(&state, 2), PairStatus::Pending);
}

#[test]
fn late_rows_for_a_replaced_pair_id_are_dropped() {
    let state = finished_three();
    let (state, _) = act(state, CompareAction::Start);
    assert!(state.compare.results.is_empty());
    let (state, cmds) = update(state, rows_for(4, 1));
    assert!(cmds.is_empty() && state.compare.results.is_empty());
    let state = feed(state, [completed(2, 7)]);
    assert_eq!(
        state.compare.results.keys().copied().collect::<Vec<_>>(),
        vec![7]
    );
    let state = update(state, rows_for(4, 1)).0;
    assert!(state.compare.results[&7].rows.is_empty());
}

#[test]
fn late_detail_after_switching_pair_is_dropped() {
    let state = feed(finished_three(), [rows_for(4, 1), rows_for(5, 1)]);
    let (state, _) = act(state, CompareAction::OpenPair(0));
    let key = KeyTuple(vec![Bson::Int32(0)]);
    let (state, cmds) = act(state, CompareAction::Select(key.clone()));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Detail { pair: 4, .. })]
    ));
    let (state, _) = act(state, CompareAction::OpenPair(1));
    assert!(
        state.compare.detail.is_none(),
        "switching pair clears the detail"
    );
    let late = |pair| {
        ev(CompareEvent::Detail {
            session: 1,
            pair,
            key: key.clone(),
            source: None,
            target: None,
            changes: vec![],
        })
    };
    let state = update(state, late(4)).0;
    assert!(state.compare.detail.is_none());
    // Even with the same key loading on the other pair, pair 4's answer is not its own.
    let (state, _) = act(state, CompareAction::Select(key.clone()));
    let state = update(state, late(4)).0;
    assert!(state.compare.detail.as_ref().unwrap().loading);
    let state = update(state, late(5)).0;
    assert!(!state.compare.detail.as_ref().unwrap().loading);
}
