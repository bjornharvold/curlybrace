//! Runs over several entries: fresh pair ids, one `Start`, per-pair outcomes, cancelling,
//! discarding, re-running and the late events that must not change anything.
use super::pairs::{multi_state, sources, status};
use super::*;
use crate::viewmodel::compare::DuplicatesView;
use crate::viewmodel::compare::{EntryId, PairStatus};
use curlybrace_core::compare::protocol::PairSpec;

const NAMES: [&str; 3] = ["alpha", "beta", "gamma"];

/// Checks every included entry in turn; probes are 1.. in order, so the next pair id is
/// `count + 1`.
fn checked_all(state: AppState, count: u32) -> AppState {
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
fn ready_three() -> AppState {
    checked_all(multi_state(&NAMES), 3)
}

fn started_specs(cmds: &[Command]) -> Vec<PairSpec> {
    match cmds.last() {
        Some(Command::Compare(CompareCommand::Start { pairs, .. })) => pairs.clone(),
        other => panic!("expected a Start, got {other:?}"),
    }
}

fn running_three() -> AppState {
    act(ready_three(), CompareAction::Start).0
}

fn done(run: u64, pair: u32, outcome: PairOutcome) -> Input {
    ev(CompareEvent::PairDone {
        session: 1,
        run,
        pair,
        outcome,
    })
}

fn completed(run: u64, pair: u32) -> Input {
    let summary = Summary {
        different: 1,
        ..Summary::default()
    };
    done(run, pair, PairOutcome::Completed(summary))
}

fn failed_with(run: u64, pair: u32, error: CoreError) -> Input {
    done(run, pair, PairOutcome::Failed(error))
}

fn run_done_of(run: u64) -> Input {
    ev(CompareEvent::RunDone { session: 1, run })
}

fn rows_for(pair: u32, n: i32) -> Input {
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

fn feed(state: AppState, inputs: impl IntoIterator<Item = Input>) -> AppState {
    inputs.into_iter().fold(state, |s, i| update(s, i).0)
}

/// All three pairs of run 1 completed and the run over.
fn finished_three() -> AppState {
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

fn pair_of(state: &AppState, entry: EntryId) -> u32 {
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

#[test]
fn rerun_discards_the_old_pair_id_and_keeps_other_results() {
    let state = finished_three();
    let (state, cmds) = act(state, CompareAction::Rerun(1));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Discard {
            session: 1,
            pair: 5
        })]
    ));
    let held: Vec<u32> = state.compare.results.keys().copied().collect();
    assert_eq!(held, vec![4, 6]);
    assert_eq!(status(&state, 1), PairStatus::Discarded);
    let included: Vec<bool> = state
        .compare
        .pairs
        .entries
        .iter()
        .map(|e| e.included)
        .collect();
    assert_eq!(included, vec![false, true, false]);
    assert_eq!(state.compare.pairs.editing, Some(1));
    assert!(state.compare.open_pair.is_none() && state.compare.detail.is_none());
    assert!(matches!(status(&state, 0), PairStatus::Completed(_)));
    assert!(
        !state.compare.can_start(),
        "the verdict is gone until checked again"
    );
    // Checking and running only that entry assigns it a new pair and leaves the others.
    let state = checked_all(state, 1);
    let (state, cmds) = act(state, CompareAction::Start);
    let ids: Vec<u32> = started_specs(&cmds).iter().map(|s| s.id).collect();
    assert_eq!(ids, vec![8], "probe 7 was the check");
    assert_eq!(pair_of(&state, 0), 4);
}

#[test]
fn rerun_is_refused_while_the_entry_is_running() {
    let (state, cmds) = act(running_three(), CompareAction::Rerun(1));
    assert!(cmds.is_empty());
    assert!(state.compare.pairs.entries.iter().all(|e| e.included));
}

#[test]
fn rerun_is_refused_while_another_pair_of_the_run_is_still_in_flight() {
    let state = update(running_three(), completed(1, 4)).0;
    let (state, cmds) = act(state, CompareAction::Rerun(0));
    assert!(cmds.is_empty());
    assert!(state.compare.pairs.entries.iter().all(|e| e.included));
    assert_eq!(state.compare.pairs.editing, None);
    assert!(matches!(status(&state, 0), PairStatus::Completed(_)));
}

#[test]
fn rerun_is_refused_while_a_check_is_in_flight() {
    let (state, _) = act(finished_three(), CompareAction::RunCheck);
    assert!(state.compare.check_in_flight());
    let (state, cmds) = act(state, CompareAction::Rerun(1));
    assert!(cmds.is_empty());
    assert_eq!(state.compare.pairs.editing, None);
    assert!(matches!(status(&state, 1), PairStatus::Completed(_)));
}

#[test]
fn discard_pair_sends_discard_for_the_rows_current_pair_id() {
    let state = finished_three();
    let (state, _) = act(state, CompareAction::OpenPair(1));
    let (state, cmds) = act(state, CompareAction::DiscardPair(1));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Discard {
            session: 1,
            pair: 5
        })]
    ));
    assert_eq!(status(&state, 1), PairStatus::Discarded);
    assert!(!state.compare.results.contains_key(&5));
    assert!(state.compare.open_pair.is_none() && state.compare.detail.is_none());
    assert_eq!(state.compare.results.len(), 2, "the others keep theirs");
    let (_, cmds) = act(state, CompareAction::DiscardPair(1));
    assert!(cmds.is_empty(), "nothing left to discard");
}

#[test]
fn a_pair_still_queued_cannot_be_discarded() {
    let (_, cmds) = act(running_three(), CompareAction::DiscardPair(0));
    assert!(cmds.is_empty());
}

#[test]
fn a_failed_start_resets_only_the_pending_rows_of_that_run() {
    let state = finished_three();
    let (state, _) = act(state, CompareAction::Rerun(1));
    let state = checked_all(state, 1);
    let (state, _) = act(state, CompareAction::Start);
    assert_eq!(status(&state, 1), PairStatus::Pending);
    let refused = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_start",
        error: CoreError::CompareSetupInvalid,
    });
    let (state, _) = update(state, refused);
    assert!(!state.compare.is_running());
    assert!(matches!(
        status(&state, 1),
        PairStatus::Failed(CoreError::CompareSetupInvalid)
    ));
    assert!(matches!(status(&state, 0), PairStatus::Completed(_)));
    assert!(matches!(status(&state, 2), PairStatus::Completed(_)));
    let held: Vec<u32> = state.compare.results.keys().copied().collect();
    assert_eq!(held, vec![4, 6], "other entries keep their results");
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_start",
            ..
        })
    ));
}

#[test]
fn a_start_failure_without_a_run_only_shows_the_banner() {
    let refused = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_start",
        error: CoreError::CompareSetupInvalid,
    });
    let (state, _) = update(finished_three(), refused);
    assert!(matches!(status(&state, 0), PairStatus::Completed(_)));
    assert_eq!(state.compare.results.len(), 3);
}

#[test]
fn remove_pair_is_refused_while_its_row_is_pending_or_running() {
    let state = running_three();
    let (state, cmds) = act(state, CompareAction::RemovePair(0));
    assert!(cmds.is_empty() && state.compare.pairs.entries.len() == 3);
    let progress = ev(CompareEvent::Progress {
        session: 1,
        run: 1,
        pair: 5,
        scanned_source: 1,
        scanned_target: 1,
        differences: 0,
    });
    let state = update(state, progress).0;
    let (state, cmds) = act(state, CompareAction::RemovePair(1));
    assert!(cmds.is_empty() && state.compare.pairs.entries.len() == 3);
    // Once its pair has ended, the entry can go (and takes its results along).
    let state = update(state, completed(1, 5)).0;
    let (state, cmds) = act(state, CompareAction::RemovePair(1));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Discard { pair: 5, .. })]
    ));
    assert_eq!(sources(&state), vec!["alpha", "gamma"]);
}

#[test]
fn pair_done_requires_the_matching_active_run() {
    // The run ended without terminals for its rows; a late PairDone must not settle them.
    let state = update(running_three(), run_done_of(1)).0;
    let (state, cmds) = update(state, completed(1, 4));
    assert!(cmds.is_empty() && state.compare.results.is_empty());
    assert!(
        matches!(status(&state, 0), PairStatus::Failed(_)),
        "ended by RunDone"
    );
    // Nor may an event of another run settle the active run's row.
    let (state, _) = act(finished_three(), CompareAction::Start);
    let (state, cmds) = update(state, completed(1, 7));
    assert!(cmds.is_empty());
    assert_eq!(status(&state, 0), PairStatus::Pending);
}

#[test]
fn pair_done_keeps_the_open_detail_unless_the_open_pair_changes() {
    let state = running_three();
    let state = feed(state, [completed(1, 4), rows_for(4, 1)]);
    // Two pairs run, so nothing opened by itself.
    assert!(state.compare.open_pair.is_none());
    let (state, _) = act(state, CompareAction::OpenPair(0));
    let key = KeyTuple(vec![Bson::Int32(0)]);
    let (state, _) = act(state, CompareAction::Select(key));
    assert!(state.compare.detail.is_some());
    let state = update(state, completed(1, 5)).0;
    assert_eq!(
        state.compare.open_pair,
        Some(4),
        "another pair finishing opens nothing"
    );
    assert!(
        state.compare.detail.is_some(),
        "and leaves the detail alone"
    );
    let state = update(state, failed_with(1, 6, CoreError::CompareKeyMissing)).0;
    assert!(state.compare.detail.is_some());
}

#[test]
fn a_run_of_one_pair_opens_its_results_when_nothing_is_open() {
    let (state, _) = act(
        checked_all(multi_state(&["alpha"]), 1),
        CompareAction::Start,
    );
    let (state, cmds) = update(state, completed(1, 2));
    assert_eq!(state.compare.open_pair, Some(2));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows { pair: 2, .. })]
    ));
}

#[test]
fn open_pair_needs_a_completed_row_and_switching_clears_the_detail() {
    let (_, cmds) = act(running_three(), CompareAction::OpenPair(0));
    assert!(cmds.is_empty());
    let state = feed(finished_three(), [rows_for(4, 1), rows_for(5, 1)]);
    let (state, _) = act(state, CompareAction::OpenPair(0));
    assert_eq!(state.compare.open_pair, Some(4));
    let key = KeyTuple(vec![Bson::Int32(0)]);
    let (state, _) = act(state, CompareAction::Select(key));
    let (same, _) = act(copy_of(&state), CompareAction::OpenPair(0));
    assert!(
        same.compare.detail.is_some(),
        "the same pair keeps its detail"
    );
    let (other, _) = act(state, CompareAction::OpenPair(1));
    assert_eq!(other.compare.open_pair, Some(5));
    assert!(other.compare.detail.is_none());
    let (back, _) = act(other, CompareAction::BackToOverview);
    assert!(back.compare.open_pair.is_none() && back.compare.detail.is_none());
}

#[test]
fn open_pair_asks_again_for_a_page_that_never_arrived() {
    let state = finished_three();
    let (state, _) = act(state, CompareAction::OpenPair(0));
    let (state, cmds) = act(state, CompareAction::OpenPair(1));
    assert!(cmds.is_empty(), "the first page is still on its way");
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_rows",
        error: CoreError::NotFound,
    });
    let (state, _) = update(state, failed);
    let (state, cmds) = act(state, CompareAction::RetryRows);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows { pair: 5, .. })]
    ));
    drop(state);
}

#[test]
fn rows_for_a_pair_without_results_are_dropped() {
    let state = finished_three();
    let (state, cmds) = update(state, rows_for(99, 2));
    assert!(cmds.is_empty() && state.compare.results.len() == 3);
}

#[test]
fn run_done_without_cancel_ends_lost_rows_so_remove_and_rerun_work() {
    let state = feed(running_three(), [completed(1, 4), run_done_of(1)]);
    assert!(!state.compare.is_running());
    for entry in [1, 2] {
        assert_eq!(
            status(&state, entry),
            PairStatus::Failed(CoreError::Database("internal error".into()))
        );
    }
    let (after, _) = act(copy_of(&state), CompareAction::RemovePair(1));
    assert_eq!(sources(&after), vec!["alpha", "gamma"]);
    let (after, _) = act(state, CompareAction::Rerun(2));
    assert_eq!(after.compare.pairs.editing, Some(2));
}

#[test]
fn rows_failure_marks_a_non_open_pairs_first_page_and_opening_it_retries() {
    // Two pairs complete; their first pages are outstanding; nothing is open.
    let state = feed(running_three(), [completed(1, 4), completed(1, 5)]);
    assert!(state.compare.open_pair.is_none());
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_rows",
        error: CoreError::NotFound,
    });
    let (state, _) = update(state, failed);
    assert!(state.compare.results[&4].load_failed && state.compare.results[&5].load_failed);
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_rows",
            ..
        })
    ));
    let (state, _) = act(state, CompareAction::OpenPair(1));
    let (state, cmds) = act(state, CompareAction::RetryRows);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows { pair: 5, .. })]
    ));
    assert!(!state.compare.results[&5].load_failed);
    assert!(
        state.compare.results[&4].load_failed,
        "the other keeps its failure"
    );
}

#[test]
fn a_rows_failure_with_nothing_outstanding_raises_no_banner() {
    // Our own Discard raced the first-page request: no view waits for a page any more.
    let state = feed(running_three(), [completed(1, 4)]);
    let (state, _) = act(state, CompareAction::DiscardPair(0));
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_rows",
        error: CoreError::NotFound,
    });
    let (state, _) = update(state, failed);
    assert!(state.banner.is_none());
}

#[test]
fn start_passing_is_not_blocked_by_an_invalid_included_entry() {
    use crate::viewmodel::compare::PairOverrides;
    let state = multi_state(&NAMES);
    let broken = PairOverrides {
        key_text: Some("a,,b".into()),
        ..PairOverrides::default()
    };
    let (state, _) = act(state, CompareAction::SetPairOverrides(1, broken));
    let first = state.compare.next_pair;
    let (mut state, _) = act(state, CompareAction::RunCheck);
    for probe in first..first + 2 {
        state = update(state, checked_as(probe, indexed(5))).0;
    }
    let (state, cmds) = act(state, CompareAction::Start);
    assert!(
        cmds.is_empty(),
        "Run still needs every included entry to pass"
    );
    let (_, cmds) = act(state, CompareAction::StartPassing);
    let names: Vec<String> = started_specs(&cmds)
        .iter()
        .map(|s| s.source.collection.clone())
        .collect();
    assert_eq!(names, vec!["alpha", "gamma"]);
}

#[test]
fn the_auto_open_rule_counts_the_pairs_the_run_started() {
    // Three pairs started: the first finishing must not open itself, whatever the rows say.
    let (state, _) = update(running_three(), completed(1, 4));
    assert!(state.compare.open_pair.is_none());
    // Even after the other two rows left (removed) the run is still a three-pair run.
    let (state, _) = update(state, completed(1, 5));
    let (state, _) = act(state, CompareAction::RemovePair(1));
    let (state, _) = act(state, CompareAction::RemovePair(0));
    assert_eq!(state.compare.overview.rows.len(), 1, "one row is left");
    let (state, _) = update(state, completed(1, 6));
    assert!(state.compare.open_pair.is_none());
}

#[test]
fn start_clears_the_open_pair_and_detail_when_the_open_pair_is_superseded() {
    let state = feed(finished_three(), [rows_for(4, 1)]);
    let (state, _) = act(state, CompareAction::OpenPair(0));
    let (state, _) = act(state, CompareAction::Select(KeyTuple(vec![Bson::Int32(0)])));
    assert!(state.compare.detail.is_some());
    let (state, _) = act(state, CompareAction::Start);
    assert!(state.compare.open_pair.is_none() && state.compare.detail.is_none());
}

#[test]
fn start_keeps_an_open_pair_that_is_not_superseded() {
    let state = feed(finished_three(), [rows_for(5, 1)]);
    let (state, _) = act(state, CompareAction::OpenPair(1));
    let (state, _) = act(state, CompareAction::Rerun(0));
    // Rerun leaves the overview (open pair cleared), so open it again before the run.
    let (state, _) = act(state, CompareAction::OpenPair(1));
    let state = checked_all(state, 1);
    let (state, _) = act(state, CompareAction::Start);
    assert_eq!(
        state.compare.open_pair,
        Some(5),
        "entry 1 was not part of the run"
    );
}

#[test]
fn run_check_is_refused_while_an_orphaned_check_is_outstanding() {
    let (state, _) = act(multi_state(&NAMES), CompareAction::RunCheck);
    let (state, _) = act(state, CompareAction::SetKey("sku".into()));
    assert!(state.compare.check_in_flight(), "the orphan is still out");
    let (state, cmds) = act(state, CompareAction::RunCheck);
    assert!(cmds.is_empty());
    let (state, _) = update(state, checked_as(1, indexed(5)));
    let (_, cmds) = act(state, CompareAction::RunCheck);
    assert_eq!(cmds.len(), 1, "the orphan's answer unlocks checking again");
}

#[test]
fn removing_or_discarding_another_entry_keeps_a_duplicates_panel() {
    let failures = [
        done(1, 4, PairOutcome::Failed(CoreError::CompareDuplicateKey)),
        completed(1, 5),
        completed(1, 6),
        run_done_of(1),
    ];
    let state = feed(running_three(), failures);
    let (loading, _) = act(state, CompareAction::FindDuplicates(0));
    assert!(matches!(
        loading.compare.duplicates,
        DuplicatesView::Loading {
            entry: 0,
            stale: false,
            ..
        }
    ));
    let (kept, _) = act(copy_of(&loading), CompareAction::DiscardPair(1));
    assert_eq!(kept.compare.duplicates.entry(), Some(0));
    assert!(matches!(
        kept.compare.duplicates,
        DuplicatesView::Loading { stale: false, .. }
    ));
    let (kept, _) = act(copy_of(&loading), CompareAction::RemovePair(2));
    assert!(matches!(
        kept.compare.duplicates,
        DuplicatesView::Loading { stale: false, .. }
    ));
    // Its own entry going away does supersede the lookup.
    let (gone, _) = act(loading, CompareAction::RemovePair(0));
    assert!(matches!(
        gone.compare.duplicates,
        DuplicatesView::Loading { stale: true, .. }
    ));
}
