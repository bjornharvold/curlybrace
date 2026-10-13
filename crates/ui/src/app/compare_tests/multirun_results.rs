//! Compare reducer: re-run, discard, remove, open pair and result paging across several pairs
//! (continued from `multirun.rs`).
use super::multirun::*;
use super::pairs::{multi_state, sources, status};
use super::*;
use crate::viewmodel::compare::{DuplicatesView, PairStatus};

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
    // Neither can it while another pair of the run is still live, even a finished entry.
    let state = update(state, completed(1, 5)).0;
    let (state, cmds) = act(state, CompareAction::RemovePair(1));
    assert!(cmds.is_empty() && state.compare.pairs.entries.len() == 3);
    // Once the run is over, the entry can go (and takes its results along).
    let state = feed(state, [completed(1, 4), completed(1, 6), run_done_of(1)]);
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
fn run_and_run_passing_skip_an_invalid_included_entry() {
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
    // Run covers the valid entries (each passed); the invalid one is skipped.
    for action in [CompareAction::Start, CompareAction::StartPassing] {
        let (_, cmds) = act(copy_of(&state), action);
        let names: Vec<String> = started_specs(&cmds)
            .iter()
            .map(|s| s.source.collection.clone())
            .collect();
        assert_eq!(names, vec!["alpha", "gamma"]);
    }
}

#[test]
fn the_auto_open_rule_counts_the_pairs_the_run_started() {
    // Three pairs started: the first finishing must not open itself, whatever the rows say.
    let (state, _) = update(running_three(), completed(1, 4));
    assert!(state.compare.open_pair.is_none());
    // Rows cannot leave mid-run, so the last one finishing is still a three-pair run.
    let (state, _) = update(state, completed(1, 5));
    let (state, _) = act(state, CompareAction::RemovePair(1));
    assert_eq!(state.compare.overview.rows.len(), 3, "removal is refused");
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
