use super::*;
use bson::Bson;
use curlybrace_core::compare::apply::{ApplyReport, IdPolicy};
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::PairOutcome;
use curlybrace_core::compare::results::Category;
use curlybrace_core::store::Namespace;

use super::super::pairs::PairList;
use super::super::pending::Scope;
use super::super::results::ResultsView;
use super::super::review::{KindCounts, ReviewPair};
use super::super::selection::RowAction;
use super::super::{ActiveRun, CheckQueue, PendingList, RunOverview};
use super::super::{CompareState, SessionPhase};

fn review(pairs: &[(PairId, u64)]) -> Review {
    Review {
        pairs: pairs
            .iter()
            .map(|(pair, n)| ReviewPair {
                pair: *pair,
                entry: *pair as EntryId + 100,
                destinations: vec![],
                counts: KindCounts {
                    insert: *n,
                    ..KindCounts::default()
                },
            })
            .collect(),
        id_policy: IdPolicy::Keep,
        confirm_text: String::new(),
        required: None,
        warnings: vec![],
        warn_inputs: vec![],
    }
}

fn report(applied: u64, stale: u64, failed: u64, not_run: u64) -> ApplyReport {
    ApplyReport {
        applied,
        stale,
        failed,
        not_run,
        ..ApplyReport::default()
    }
}

fn running() -> ApplyUi {
    ApplyUi::reviewing(review(&[(1, 10), (2, 5)])).started(7)
}

fn rows(ui: &ApplyUi) -> &[ApplyRow] {
    match ui {
        ApplyUi::Running { rows, .. } => rows,
        other => panic!("not running: {other:?}"),
    }
}

#[test]
fn started_turns_the_reviewed_pairs_into_rows() {
    let ui = running();
    assert!(ui.is_busy());
    assert!(ui.is_running());
    assert_eq!(rows(&ui).len(), 2);
    assert_eq!(
        (rows(&ui)[0].pair, rows(&ui)[0].total, rows(&ui)[0].done),
        (1, 10, 0)
    );
    match &ui {
        ApplyUi::Running {
            run,
            total,
            cancel_requested,
            ..
        } => {
            assert_eq!((*run, *total, *cancel_requested), (7, 15, false));
        }
        _ => unreachable!(),
    }
}

#[test]
fn only_reviewing_can_start_and_idle_is_not_busy() {
    assert_eq!(ApplyUi::Idle.started(1), ApplyUi::Idle);
    assert!(!ApplyUi::Idle.is_busy());
    assert!(ApplyUi::reviewing(review(&[(1, 1)])).is_busy());
    assert!(
        !ApplyUi::Finished {
            reports: vec![],
            cancelled: false,
            ops_sent: 0
        }
        .is_busy()
    );
}

#[test]
fn progress_folds_for_the_right_run_and_pair_only() {
    let ui = running().progressed(7, 2, 3, 5);
    assert_eq!(rows(&ui)[1].done, 3);
    assert_eq!(rows(&ui)[0].done, 0);
    let stale_run = ui.clone().progressed(6, 1, 9, 10);
    assert_eq!(stale_run, ui);
    let unknown = ui.clone().progressed(7, 99, 9, 10);
    assert_eq!(unknown, ui);
    assert_eq!(ApplyUi::Idle.progressed(7, 1, 1, 1), ApplyUi::Idle);
}

#[test]
fn pair_done_stores_the_report_for_the_right_run_and_pair_only() {
    let ui = running().pair_done(7, 1, report(10, 0, 0, 0));
    assert_eq!(rows(&ui)[0].report, Some(report(10, 0, 0, 0)));
    assert_eq!(rows(&ui)[0].done, 10);
    assert_eq!(ui.clone().pair_done(6, 2, report(5, 0, 0, 0)), ui);
    assert_eq!(ui.clone().pair_done(7, 99, report(5, 0, 0, 0)), ui);
}

#[test]
fn apply_done_finishes_with_totals_and_ignores_other_runs() {
    let ui = running()
        .pair_done(7, 1, report(6, 2, 1, 1))
        .pair_done(7, 2, report(5, 0, 0, 0));
    assert_eq!(ui.clone().apply_done(6), ui);
    match ui.apply_done(7) {
        ApplyUi::Finished {
            reports,
            cancelled,
            ops_sent,
        } => {
            assert_eq!(reports.len(), 2);
            assert!(!cancelled);
            assert_eq!(ops_sent, 15);
            let sum: u64 = reports
                .iter()
                .map(|(_, r)| r.applied + r.stale + r.not_planned + r.failed + r.not_run)
                .sum();
            assert_eq!(sum, ops_sent);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_cancelled_apply_is_flagged_and_pairs_without_a_report_are_left_out() {
    let ui = running().cancelling().pair_done(7, 1, report(4, 0, 0, 6));
    assert!(!ui.can_cancel());
    match ui.apply_done(7) {
        ApplyUi::Finished {
            reports,
            cancelled,
            ops_sent,
        } => {
            assert!(cancelled);
            assert_eq!(reports.len(), 1);
            assert_eq!(ops_sent, 10);
        }
        other => panic!("{other:?}"),
    }
    assert!(running().can_cancel());
    assert_eq!(ApplyUi::Idle.cancelling(), ApplyUi::Idle);
}

#[test]
fn touched_pairs_are_those_that_wrote_or_failed() {
    let ui = running()
        .pair_done(7, 1, report(0, 3, 0, 0))
        .pair_done(7, 2, report(0, 0, 2, 0))
        .apply_done(7);
    assert_eq!(ui.touched_pairs(), vec![2]);
    assert!(running().touched_pairs().is_empty());
}

#[test]
fn typing_only_changes_a_review_and_dismiss_keeps_a_running_apply() {
    let ui = ApplyUi::reviewing(review(&[(1, 1)])).confirm_typed("abc");
    assert_eq!(ui.review().unwrap().confirm_text, "abc");
    assert_eq!(running().confirm_typed("x"), running());
    assert_eq!(ui.dismissed(), ApplyUi::Idle);
    assert_eq!(running().dismissed(), running());
}

// ---- overview marker and gates

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn summary() -> Summary {
    Summary {
        only_in_source: 2,
        ..Summary::default()
    }
}

/// Ready session, one entry (id 0) whose run (pair 5) completed with 2 only-in-source rows,
/// and one pending copy of key 1.
fn reviewable() -> CompareState {
    let pairs = PairList::default().added(
        Namespace {
            db: "a".into(),
            collection: "x".into(),
        },
        Namespace {
            db: "b".into(),
            collection: "x".into(),
        },
    );
    let (overview, _) = RunOverview::default().started(1, &[(0, 5)]);
    let overview = overview
        .pair_done(1, 5, PairOutcome::Completed(summary()))
        .run_done(1);
    let (pending, _) = PendingList::default().with_action(
        5,
        Category::OnlyInSource,
        RowAction::CopyToTarget,
        Scope::Keys(vec![k(1)]),
    );
    CompareState {
        session: Some(SessionPhase::Ready(1)),
        connections: Some((uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2))),
        pairs,
        overview,
        results: [(5, ResultsView::new(5, &summary()))].into(),
        pending,
        ..CompareState::default()
    }
}

#[test]
fn marked_applied_flags_the_pair_and_a_new_run_clears_it() {
    let s = reviewable();
    assert!(!s.overview.rows[0].applied);
    let o = s.overview.marked_applied(5);
    assert!(o.rows[0].applied);
    let untouched = o.clone().marked_applied(99);
    assert_eq!(untouched, o);
    let (o, _) = o.started(2, &[(0, 6)]);
    assert!(!o.rows[0].applied);
}

#[test]
fn review_is_allowed_when_everything_lines_up() {
    let s = reviewable();
    assert!(s.can_review());
    assert!(s.gates(&s.digest()).review);
}

#[test]
fn review_is_blocked_in_each_blocking_situation() {
    let base = reviewable();
    let cases: Vec<(&str, CompareState)> = vec![
        (
            "no pending",
            CompareState {
                pending: Default::default(),
                ..base.clone()
            },
        ),
        (
            "pair applied",
            CompareState {
                overview: base.overview.clone().marked_applied(5),
                ..base.clone()
            },
        ),
        (
            "pair not completed",
            CompareState {
                overview: base.overview.clone().discarded(5),
                ..base.clone()
            },
        ),
        (
            "unknown pair",
            CompareState {
                overview: Default::default(),
                ..base.clone()
            },
        ),
        (
            "run active",
            CompareState {
                overview: RunOverview {
                    active: Some(ActiveRun {
                        run: 9,
                        cancel_requested: false,
                        failure_noted: false,
                        pairs: 1,
                    }),
                    ..base.overview.clone()
                },
                ..base.clone()
            },
        ),
        (
            "check queued",
            CompareState {
                checks: CheckQueue::queued(&[0]),
                ..base.clone()
            },
        ),
        (
            "orphan check",
            CompareState {
                orphaned_check: Some(3),
                ..base.clone()
            },
        ),
        (
            "apply busy",
            CompareState {
                apply: running(),
                ..base.clone()
            },
        ),
        (
            "reviewing",
            CompareState {
                apply: ApplyUi::reviewing(review(&[(5, 1)])),
                ..base.clone()
            },
        ),
        (
            "no session",
            CompareState {
                session: None,
                ..base.clone()
            },
        ),
        (
            "opening",
            CompareState {
                session: Some(SessionPhase::Opening(1)),
                ..base.clone()
            },
        ),
    ];
    for (name, state) in cases {
        assert!(!state.can_review(), "{name}");
    }
}

#[test]
fn rerun_remove_and_discard_wait_while_an_apply_is_busy() {
    let idle = reviewable();
    assert!(idle.can_rerun(0));
    assert!(idle.can_remove(0));
    assert!(idle.can_discard());
    assert!(idle.overview_row(0).unwrap().can_discard);
    for apply in [running(), ApplyUi::reviewing(review(&[(5, 1)]))] {
        let busy = CompareState {
            apply,
            ..idle.clone()
        };
        assert!(!busy.can_rerun(0));
        assert!(!busy.can_remove(0));
        assert!(!busy.can_discard());
        assert!(!busy.overview_row(0).unwrap().can_discard);
        assert!(!busy.can_start());
        assert!(!busy.can_check());
    }
}

#[test]
fn the_state_builds_a_review_from_its_entries_and_results() {
    let s = reviewable();
    let r = s
        .review(IdPolicy::Keep, &|id| format!("conn-{}", id.as_u128()))
        .expect("a review");
    assert_eq!(r.pairs.len(), 1);
    assert_eq!(r.pairs[0].entry, 0);
    assert_eq!(r.pairs[0].destinations[0].1.connection, "conn-2");
    assert_eq!(r.pairs[0].destinations[0].1.database, "b");
    assert_eq!(r.pairs[0].counts.insert, 1);
    let blocked = CompareState {
        apply: running(),
        ..s
    };
    assert!(blocked.review(IdPolicy::Keep, &|_| String::new()).is_none());
}

#[test]
fn a_dropped_session_forgets_the_apply() {
    let s = CompareState {
        apply: running(),
        ..reviewable()
    }
    .session_dropped();
    assert_eq!(s.apply, ApplyUi::Idle);
}
