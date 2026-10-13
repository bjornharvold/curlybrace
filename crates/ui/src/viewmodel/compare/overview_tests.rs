use super::*;

fn summary(different: u64, only_source: u64) -> Summary {
    Summary {
        different,
        only_in_source: only_source,
        equal: 3,
        scanned_source: 10,
        scanned_target: 9,
        ..Summary::default()
    }
}

fn running(run: RunId, pairs: &[(EntryId, PairId)]) -> RunOverview {
    RunOverview::default().started(run, pairs).0
}

fn status_of(o: &RunOverview, entry: EntryId) -> PairStatus {
    o.row_for(entry).expect("row").status.clone()
}

#[test]
fn started_replaces_rows_of_rerun_entries_and_returns_the_old_pair_ids() {
    let done = running(1, &[(1, 10), (2, 11)])
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)))
        .pair_done(1, 11, PairOutcome::Failed(CoreError::CompareDuplicateKey))
        .run_done(1);
    let (next, superseded) = done.started(2, &[(2, 20)]);
    assert_eq!(superseded, vec![11]);
    let row = next.row_for(2).unwrap();
    assert_eq!(
        (row.pair, row.run, &row.status),
        (20, 2, &PairStatus::Pending)
    );
    assert_eq!(
        next.active,
        Some(ActiveRun {
            run: 2,
            cancel_requested: false
        })
    );
}

#[test]
fn started_keeps_rows_of_entries_not_in_the_run() {
    let done = running(1, &[(1, 10), (2, 11)])
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)))
        .pair_done(1, 11, PairOutcome::Completed(summary(0, 2)))
        .run_done(1);
    let (next, superseded) = done.started(2, &[(2, 20)]);
    assert_eq!(superseded, vec![11]);
    assert_eq!(next.rows.len(), 2);
    assert!(matches!(status_of(&next, 1), PairStatus::Completed(_)));
    assert_eq!(next.row_for(1).unwrap().pair, 10);
}

#[test]
fn started_does_not_return_pairs_that_were_already_discarded() {
    let done = running(1, &[(1, 10)])
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)))
        .run_done(1)
        .discarded(10);
    let (_, superseded) = done.started(2, &[(1, 20)]);
    assert!(superseded.is_empty());
}

#[test]
fn progress_for_another_run_or_pair_is_ignored() {
    let o = running(2, &[(1, 10)]);
    assert_eq!(o.clone().progressed(1, 10, (5, 5, 1)), o);
    assert_eq!(o.clone().progressed(2, 99, (5, 5, 1)), o);
    let moved = o.progressed(2, 10, (5, 6, 1));
    assert_eq!(
        status_of(&moved, 1),
        PairStatus::Running {
            scanned_source: 5,
            scanned_target: 6,
            differences: 1
        }
    );
}

#[test]
fn progress_after_the_run_is_done_is_ignored() {
    let o = running(1, &[(1, 10)]).run_done(1);
    assert_eq!(o.clone().progressed(1, 10, (5, 5, 1)), o);
}

#[test]
fn pair_done_for_a_wrong_pair_id_is_ignored() {
    let o = running(1, &[(1, 10)]);
    let outcome = PairOutcome::Completed(summary(1, 0));
    assert_eq!(o.clone().pair_done(1, 99, outcome.clone()), o);
    assert_eq!(o.clone().pair_done(2, 10, outcome.clone()), o);
    let done = o.pair_done(1, 10, outcome);
    assert!(matches!(status_of(&done, 1), PairStatus::Completed(_)));
}

#[test]
fn a_terminal_row_does_not_change_again() {
    let o = running(1, &[(1, 10)]).pair_done(1, 10, PairOutcome::Cancelled(summary(0, 0)));
    let again = o
        .clone()
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)));
    assert_eq!(again, o);
}

#[test]
fn a_failing_pair_leaves_its_neighbours_untouched() {
    let o = running(1, &[(1, 10), (2, 11), (3, 12)])
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)))
        .pair_done(1, 11, PairOutcome::Failed(CoreError::CompareDuplicateKey))
        .pair_done(1, 12, PairOutcome::Completed(summary(0, 4)));
    assert!(matches!(status_of(&o, 1), PairStatus::Completed(_)));
    assert_eq!(
        status_of(&o, 2),
        PairStatus::Failed(CoreError::CompareDuplicateKey)
    );
    assert!(matches!(status_of(&o, 3), PairStatus::Completed(_)));
}

#[test]
fn run_done_clears_active() {
    let o = running(1, &[(1, 10)]);
    assert!(o.is_running());
    assert_eq!(
        o.clone().run_done(2),
        o,
        "another run's terminal is ignored"
    );
    let done = o.run_done(1);
    assert!(!done.is_running() && done.active.is_none());
    assert_eq!(status_of(&done, 1), PairStatus::Pending, "untouched");
}

#[test]
fn a_cancelled_run_marks_the_pairs_it_never_reached_as_cancelled() {
    let o = running(1, &[(1, 10), (2, 11)])
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)))
        .cancelling()
        .run_done(1);
    assert!(matches!(status_of(&o, 1), PairStatus::Completed(_)));
    assert_eq!(status_of(&o, 2), PairStatus::Cancelled(Summary::default()));
}

#[test]
fn totals_sum_completed_pairs_only() {
    let o = running(1, &[(1, 10), (2, 11), (3, 12), (4, 13)])
        .pair_done(1, 10, PairOutcome::Completed(summary(2, 1)))
        .pair_done(1, 11, PairOutcome::Completed(summary(5, 0)))
        .pair_done(1, 12, PairOutcome::Cancelled(summary(9, 9)))
        .pair_done(1, 13, PairOutcome::Failed(CoreError::CompareDuplicateKey));
    let t = o.totals();
    assert_eq!((t.different, t.only_in_source, t.only_in_target), (7, 1, 0));
    assert_eq!(t.identical, 6);
    assert_eq!((t.completed, t.cancelled, t.failed), (2, 1, 1));
}

#[test]
fn cancel_marks_cancel_requested_once() {
    let o = running(1, &[(1, 10)]);
    assert!(o.can_cancel());
    let o = o.cancelling();
    assert!(o.active.unwrap().cancel_requested);
    assert!(!o.can_cancel());
    assert_eq!(o.clone().cancelling(), o);
    let idle = RunOverview::default();
    assert!(!idle.can_cancel());
    assert_eq!(idle.clone().cancelling(), idle);
}

#[test]
fn discarded_only_applies_to_terminal_rows() {
    let o = running(1, &[(1, 10), (2, 11)]).pair_done(1, 10, PairOutcome::Completed(summary(1, 0)));
    let o = o.discarded(10).discarded(11);
    assert_eq!(status_of(&o, 1), PairStatus::Discarded);
    assert_eq!(status_of(&o, 2), PairStatus::Pending);
    assert_eq!(o.clone().discarded(99), o);
}

#[test]
fn removed_entry_returns_the_pair_id_that_needs_a_discard() {
    let o = running(1, &[(1, 10), (2, 11), (3, 12)])
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)))
        .pair_done(1, 11, PairOutcome::Completed(summary(1, 0)))
        .discarded(11)
        .run_done(1);
    let (o, pair) = o.removed_entry(1);
    assert_eq!(pair, Some(10));
    assert!(o.row_for(1).is_none());
    let (o, pair) = o.removed_entry(2);
    assert_eq!(pair, None, "already discarded");
    assert!(o.row_for(2).is_none());
    let (kept, pair) = o.clone().removed_entry(9);
    assert_eq!((kept, pair), (o, None));
}

#[test]
fn any_settled_ignores_live_and_discarded_rows() {
    let o = running(1, &[(1, 10)]);
    assert!(!o.any_settled());
    let done = o.pair_done(1, 10, PairOutcome::Completed(summary(1, 0)));
    assert!(done.any_settled());
    assert!(!done.discarded(10).any_settled());
}

#[test]
fn a_failed_pair_keeps_its_last_progress() {
    let o = running(1, &[(1, 10)])
        .progressed(1, 10, (7, 6, 2))
        .pair_done(1, 10, PairOutcome::Failed(CoreError::CompareDuplicateKey));
    assert_eq!(o.row_for(1).unwrap().last_progress, (7, 6, 2));
    let fresh = running(1, &[(1, 10)]);
    assert_eq!(fresh.row_for(1).unwrap().last_progress, (0, 0, 0));
}

#[test]
fn pair_done_needs_the_active_run() {
    // The run ended without a terminal for its rows: a late PairDone settles nothing.
    let ended = running(1, &[(1, 10)]).run_done(1);
    assert_eq!(
        ended
            .clone()
            .pair_done(1, 10, PairOutcome::Completed(summary(1, 0))),
        ended
    );
    // An event of an earlier run does not settle the active run's row either.
    let o = running(2, &[(1, 10)]);
    assert_eq!(
        o.clone()
            .pair_done(1, 10, PairOutcome::Completed(summary(1, 0))),
        o
    );
}

#[test]
fn a_refused_start_fails_only_the_pending_rows_of_the_active_run() {
    let finished = running(1, &[(1, 10), (2, 11)])
        .pair_done(1, 10, PairOutcome::Completed(summary(1, 0)))
        .pair_done(1, 11, PairOutcome::Completed(summary(0, 2)))
        .run_done(1);
    let (o, _) = finished.started(2, &[(2, 20), (3, 21)]);
    let refused = o.start_refused(&CoreError::CompareSetupInvalid);
    assert!(refused.active.is_none());
    assert!(matches!(status_of(&refused, 1), PairStatus::Completed(_)));
    for entry in [2, 3] {
        assert_eq!(
            status_of(&refused, entry),
            PairStatus::Failed(CoreError::CompareSetupInvalid)
        );
    }
}

#[test]
fn a_refused_start_keeps_the_rows_of_the_run_that_already_ended() {
    let o = running(1, &[(1, 10), (2, 11)]).pair_done(1, 10, PairOutcome::Completed(summary(1, 0)));
    let refused = o.start_refused(&CoreError::CompareSetupInvalid);
    assert!(matches!(status_of(&refused, 1), PairStatus::Completed(_)));
    assert!(matches!(status_of(&refused, 2), PairStatus::Failed(_)));
}

#[test]
fn a_refused_start_with_no_run_changes_nothing() {
    let idle = running(1, &[(1, 10)]).run_done(1);
    assert_eq!(
        idle.clone().start_refused(&CoreError::CompareSetupInvalid),
        idle
    );
}

#[test]
fn run_counters_and_live_entries() {
    let o = running(1, &[(1, 10), (2, 11), (3, 12)])
        .pair_done(1, 10, PairOutcome::Failed(CoreError::CompareDuplicateKey))
        .pair_done(1, 11, PairOutcome::Completed(summary(1, 0)));
    assert_eq!((o.rows_in(1), o.rows_in(2)), (3, 0));
    assert_eq!((o.failures_in(1), o.failures_in(2)), (1, 0));
    assert!(!o.entry_is_live(1) && !o.entry_is_live(2));
    assert!(o.entry_is_live(3), "pending counts as live");
    assert!(!o.entry_is_live(99));
}
