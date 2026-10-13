use super::*;
use bson::Bson;
use curlybrace_core::compare::apply::{ApplyReport, IdPolicy};
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::PairOutcome;
use curlybrace_core::compare::results::Category;
use curlybrace_core::store::Namespace;

use super::super::PairOverrides;
use super::super::pairs::PairList;
use super::super::pending::Scope;
use super::super::results::ResultsView;
use super::super::review::Warning;
use super::super::review::{KindCounts, ReviewPair};
use super::super::selection::RowAction;
use super::super::{ActiveRun, CheckQueue, PendingList, RunFacts, RunOverview};
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
        word: String::new(),
        session: 0,
        pending: PendingList::default(),
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
            requested: 0
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
    let done = ui.apply_done(7);
    match &done {
        ApplyUi::Finished {
            reports,
            cancelled,
            requested,
        } => {
            assert_eq!(reports.len(), 2);
            assert!(!cancelled);
            assert_eq!(*requested, 15);
        }
        other => panic!("{other:?}"),
    }
    let s = done.summary().unwrap();
    assert_eq!(
        s,
        ApplySummary {
            applied: 11,
            stale: 2,
            not_planned: 0,
            failed: 1,
            not_run: 1
        }
    );
    assert_eq!(s.total(), 15);
}

#[test]
fn a_pair_that_never_reported_counts_as_not_run() {
    let ui = running().cancelling().pair_done(7, 1, report(4, 0, 0, 6));
    assert!(!ui.can_cancel());
    let done = ui.apply_done(7);
    match &done {
        ApplyUi::Finished {
            reports,
            cancelled,
            requested,
        } => {
            assert!(cancelled);
            assert_eq!(reports.len(), 2);
            assert_eq!(reports[1].0, 2);
            assert_eq!(reports[1].1.not_run, 5);
            assert_eq!(*requested, 15);
        }
        other => panic!("{other:?}"),
    }
    let s = done.summary().unwrap();
    assert_eq!((s.applied, s.not_run), (4, 11));
    assert_eq!(s.total(), 15);
    assert!(running().can_cancel());
    assert_eq!(ApplyUi::Idle.cancelling(), ApplyUi::Idle);
    assert_eq!(ApplyUi::Idle.summary(), None);
}

#[test]
fn a_short_report_is_topped_up_so_the_summary_adds_up() {
    let done = running()
        .pair_done(7, 1, report(3, 0, 0, 0))
        .pair_done(7, 2, report(5, 0, 0, 0))
        .apply_done(7);
    let s = done.summary().unwrap();
    assert_eq!(s.total(), 15);
    assert_eq!((s.applied, s.not_run), (8, 7));
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

fn ns(db: &str) -> Namespace {
    Namespace {
        db: db.into(),
        collection: "x".into(),
    }
}

/// What the run of entry 0 used: a plain `_id` key, full scope.
fn facts() -> RunFacts {
    RunFacts {
        source: ns("a"),
        target: ns("b"),
        key_has_id: true,
        include_id: true,
        scope_is_all: true,
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
    let (overview, _) = RunOverview::default().started_with(1, &[(0, 5)], &|_| Some(facts()));
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
fn rerun_remove_discard_and_duplicates_wait_while_an_apply_is_busy() {
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
        assert!(!busy.can_discard_entry(0));
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

#[test]
fn the_review_reports_the_facts_of_the_run_not_the_edited_entry() {
    let mut s = reviewable();
    // The run used a business key and a narrow scope; the entry now says `_id`, full scope.
    let ran = RunFacts {
        key_has_id: false,
        scope_is_all: false,
        ..facts()
    };
    let (o, _) = RunOverview::default().started_with(1, &[(0, 5)], &|_| Some(ran.clone()));
    s.overview = o
        .pair_done(1, 5, PairOutcome::Completed(summary()))
        .run_done(1);
    s.pending = PendingList::default()
        .with_action(
            5,
            Category::Different,
            RowAction::CopyToTarget,
            Scope::Keys(vec![k(1)]),
        )
        .0;
    s.results = [(
        5,
        ResultsView::new(
            5,
            &Summary {
                different: 2,
                ..Summary::default()
            },
        ),
    )]
    .into();
    let r = s.review(IdPolicy::Keep, &|_| "c".into()).unwrap();
    assert_eq!(
        r.warnings,
        vec![Warning::KeepsDestinationId, Warning::WholeDocumentOverwrite]
    );
    // Now the entry is edited to a namespace the run did not use: destinations still follow the run.
    let edited = PairList::default().added(ns("zz"), ns("yy"));
    let r2 = CompareState { pairs: edited, ..s }
        .review(IdPolicy::Keep, &|_| "c".into())
        .unwrap();
    assert_eq!(r2.pairs[0].destinations[0].1.database, "b");
}

#[test]
fn an_unparsable_current_key_does_not_disable_review() {
    let s = reviewable();
    let bad = s.pairs.clone().with_overrides(
        0,
        PairOverrides {
            key_text: Some(String::new()),
            ..PairOverrides::default()
        },
    );
    let s = CompareState { pairs: bad, ..s };
    assert!(s.can_review());
    assert!(s.review(IdPolicy::Keep, &|_| String::new()).is_some());
}

#[test]
fn review_is_never_enabled_when_it_would_do_nothing() {
    let s = reviewable();
    // A pair row started without facts cannot be reviewed.
    let (o, _) = RunOverview::default().started(1, &[(0, 5)]);
    let no_facts = CompareState {
        overview: o
            .pair_done(1, 5, PairOutcome::Completed(summary()))
            .run_done(1),
        ..s.clone()
    };
    assert!(!no_facts.can_review());
    assert!(
        no_facts
            .review(IdPolicy::Keep, &|_| String::new())
            .is_none()
    );
    // A whole-category item whose results are gone.
    let (pending, _) = PendingList::default().with_action(
        5,
        Category::OnlyInSource,
        RowAction::CopyToTarget,
        Scope::Category,
    );
    let gone = CompareState {
        pending,
        results: Default::default(),
        ..s
    };
    assert!(!gone.can_review());
    assert!(gone.review(IdPolicy::Keep, &|_| String::new()).is_none());
}

#[test]
fn the_review_is_a_snapshot_of_the_names() {
    let s = reviewable();
    let names = std::cell::RefCell::new("before".to_string());
    let r = s
        .review(IdPolicy::Keep, &|_| names.borrow().clone())
        .unwrap();
    *names.borrow_mut() = "after".into();
    assert_eq!(r.pairs[0].destinations[0].1.connection, "before");
    let again = s
        .review(IdPolicy::Keep, &|_| names.borrow().clone())
        .unwrap();
    assert_eq!(again.pairs[0].destinations[0].1.connection, "after");
    assert_eq!(r.pairs[0].destinations[0].1.connection, "before");
}

/// Entry 0's run failed on a duplicate key and its check passed: Find duplicates is live.
fn duplicate_failure() -> CompareState {
    let index = curlybrace_core::store::IndexStatus::Indexed;
    let pre = curlybrace_core::compare::protocol::Preflight {
        source_count: 1,
        target_count: 1,
        source_index: index,
        target_index: index,
        unindexed_too_large: false,
    };
    let (o, _) = RunOverview::default().started(1, &[(0, 5)]);
    CompareState {
        overview: o
            .pair_done(
                1,
                5,
                PairOutcome::Failed(curlybrace_core::error::CoreError::CompareDuplicateKey),
            )
            .run_done(1),
        checks: CheckQueue::queued(&[0]).sending(0, 8).answered(8, pre).0,
        ..reviewable()
    }
}

#[test]
fn discard_and_find_duplicates_wait_while_an_apply_is_busy() {
    let idle = reviewable();
    assert!(idle.can_discard_entry(0));
    assert!(!idle.can_discard_entry(9));
    let dup = duplicate_failure();
    assert!(dup.can_find_duplicates_for(0), "live when idle");
    for apply in [running(), ApplyUi::reviewing(review(&[(5, 1)]))] {
        let busy = CompareState {
            apply: apply.clone(),
            ..idle.clone()
        };
        assert!(!busy.can_discard_entry(0));
        let busy_dup = CompareState {
            apply,
            ..dup.clone()
        };
        assert!(!busy_dup.can_find_duplicates_for(0));
    }
}
