use super::*;
use crate::viewmodel::compare::{
    CheckQueue, PairList, PairOverrides, PairStatus, RunOverview, SessionPhase, SetupForm,
};
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::protocol::{PairOutcome, Preflight, Side};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{IndexStatus, Namespace};

fn ns(db: &str, c: &str) -> Namespace {
    Namespace::new(db.to_string(), c.to_string())
}

fn state() -> CompareState {
    CompareState {
        session: Some(SessionPhase::Ready(1)),
        connections: Some((uuid::Uuid::nil(), uuid::Uuid::from_u128(1))),
        pairs: PairList::default()
            .added(ns("a", "users"), ns("b", "users"))
            .added(ns("a", "orders"), ns("b", "orders_copy")),
        ..CompareState::default()
    }
}

fn row(state: &CompareState, index: usize) -> PairRow {
    state.pair_row(index, &state.digest()).expect("row")
}

fn pre(unindexed_too_large: bool, count: u64) -> Preflight {
    let index = if unindexed_too_large {
        IndexStatus::NotIndexed
    } else {
        IndexStatus::Indexed
    };
    Preflight {
        source_count: count,
        target_count: count,
        source_index: index,
        target_index: index,
        unindexed_too_large,
    }
}

fn checked(state: CompareState, entry: u32, answer: Preflight) -> CompareState {
    let checks = CheckQueue::queued(&[entry])
        .sending(entry, 50)
        .answered(50, answer)
        .0;
    CompareState { checks, ..state }
}

// ---- icons and status keys ----

#[test]
fn every_pair_status_has_its_own_key_and_a_distinct_icon() {
    let s = Summary::default();
    let all = [
        (PairStatus::Pending, "compare.overview.status.pending"),
        (
            PairStatus::Running {
                scanned_source: 0,
                scanned_target: 0,
                differences: 0,
            },
            "compare.overview.status.running",
        ),
        (
            PairStatus::Completed(s),
            "compare.overview.status.completed",
        ),
        (
            PairStatus::Cancelled(s),
            "compare.overview.status.cancelled",
        ),
        (
            PairStatus::Failed(CoreError::NotFound),
            "compare.overview.status.failed",
        ),
        (PairStatus::Discarded, "compare.overview.status.discarded"),
    ];
    let mut icons = Vec::new();
    for (status, key) in all {
        assert_eq!(status.i18n_key(), key);
        icons.push(status.icon());
    }
    let mut unique = icons.clone();
    unique.sort_by_key(|i| i.glyph());
    unique.dedup();
    assert_eq!(unique.len(), icons.len(), "no two statuses share an icon");
}

#[test]
fn every_icon_has_a_glyph() {
    assert!(!StatusIcon::Ok.glyph().is_empty());
    assert!(!StatusIcon::Failed.glyph().is_empty());
}

// ---- pair rows ----

#[test]
fn a_pair_row_shows_both_namespaces_the_effective_key_and_no_marker_by_default() {
    let r = row(&state(), 1);
    assert_eq!(r.source, "a.orders");
    assert_eq!(r.target, "b.orders_copy");
    assert_eq!(r.key, "_id");
    assert!(r.included && !r.customised);
}

#[test]
fn a_pair_row_follows_the_default_key_until_it_overrides_it() {
    let s = state();
    let s = CompareState {
        setup: SetupForm::default().with_key_text("sku"),
        ..s
    };
    assert_eq!(row(&s, 0).key, "sku");
    let s = CompareState {
        pairs: s.pairs.with_overrides(
            0,
            PairOverrides {
                key_text: Some("email".into()),
                ..PairOverrides::default()
            },
        ),
        ..s
    };
    let r = row(&s, 0);
    assert_eq!((r.key.as_str(), r.customised), ("email", true));
    assert_eq!(row(&s, 1).key, "sku");
}

#[test]
fn an_unknown_row_index_has_no_row() {
    let s = state();
    assert!(s.pair_row(9, &s.digest()).is_none());
}

#[test]
fn entry_status_follows_the_issue_then_the_check() {
    let s = state();
    assert_eq!(row(&s, 0).status, EntryStatus::Unchecked);
    let ok = checked(s.clone(), 0, pre(false, 5));
    assert_eq!(row(&ok, 0).status, EntryStatus::Ok);
    let empty = checked(s.clone(), 0, pre(false, 0));
    assert_eq!(row(&empty, 0).status, EntryStatus::EmptyBoth);
    let blocked = checked(s.clone(), 0, pre(true, 5));
    assert_eq!(
        row(&blocked, 0).status,
        EntryStatus::NeedsIndex(Side::Source)
    );
    let checking = CompareState {
        checks: CheckQueue::queued(&[0]),
        ..s.clone()
    };
    assert_eq!(row(&checking, 0).status, EntryStatus::Checking);
    let failed = CompareState {
        checks: CheckQueue::queued(&[0]).skipped(0),
        ..s.clone()
    };
    assert_eq!(row(&failed, 0).status, EntryStatus::CheckFailed);
    let invalid = CompareState {
        setup: SetupForm::default().with_key_text("a,,b"),
        ..s
    };
    assert_eq!(
        row(&invalid, 0).status,
        EntryStatus::Invalid(crate::viewmodel::compare::SetupError::Key)
    );
}

#[test]
fn entry_status_maps_to_keys_icons_and_the_side_to_blame() {
    use EntryStatus::*;
    assert_eq!(Ok.i18n_key(), "compare.preflight.entry_ok");
    assert_eq!(EmptyBoth.i18n_key(), "compare.preflight.entry_empty");
    assert_eq!(Unchecked.i18n_key(), "compare.preflight.entry_unchecked");
    assert_eq!(Checking.i18n_key(), "compare.preflight.entry_checking");
    assert_eq!(CheckFailed.i18n_key(), "compare.preflight.entry_failed");
    assert_eq!(
        NeedsIndex(Side::Target).i18n_key(),
        "compare.preflight.entry_needs_index"
    );
    assert_eq!(
        NeedsIndex(Side::Target).side_key(),
        Some("compare.side.target")
    );
    assert_eq!(Ok.side_key(), None);
    assert_eq!(
        Invalid(crate::viewmodel::compare::SetupError::Key).i18n_key(),
        "compare.pairs.invalid"
    );
    assert_eq!(
        Invalid(crate::viewmodel::compare::SetupError::Key).hover_key(),
        Some("compare.setup.key_invalid")
    );
    assert_eq!(Ok.hover_key(), None);
    // A refusal never shares the ok icon.
    assert_ne!(Ok.icon(), NeedsIndex(Side::Source).icon());
    assert_ne!(Ok.icon(), CheckFailed.icon());
}

// ---- overview rows ----

fn summary() -> Summary {
    Summary {
        equal: 7,
        only_in_source: 1,
        only_in_target: 2,
        different: 3,
        scanned_source: 11,
        scanned_target: 12,
    }
}

fn ran(state: CompareState) -> CompareState {
    let (overview, _) = RunOverview::default().started(1, &[(0, 10), (1, 11)]);
    let overview = overview
        .pair_done(1, 10, PairOutcome::Completed(summary()))
        .progressed(1, 11, (5, 6, 1));
    CompareState { overview, ..state }
}

fn orow(state: &CompareState, index: usize) -> OverviewRow {
    state.overview_row(index).expect("row")
}

#[test]
fn a_completed_row_shows_counts_and_offers_open_discard_and_rerun() {
    let s = ran(state());
    let r = orow(&s, 0);
    assert_eq!(
        (r.source.as_str(), r.target.as_str()),
        ("a.users", "b.users")
    );
    assert_eq!(r.status_key, "compare.overview.status.completed");
    assert_eq!(
        r.detail,
        RowDetail::Counts(RowCounts {
            different: 3,
            only_in_source: 1,
            only_in_target: 2,
            identical: 7
        })
    );
    // No results view was kept: Open would do nothing.
    assert!(!r.can_open);
    // Discard is live, but Re-run waits for the run in flight to end.
    assert!(r.can_discard && !r.can_rerun);
    let over = CompareState {
        overview: s.overview.clone().run_done(1),
        ..s
    };
    assert!(orow(&over, 0).can_rerun);
}

#[test]
fn rerun_also_waits_for_a_check_in_flight() {
    let s = CompareState {
        overview: ran(state()).overview.run_done(1),
        ..state()
    };
    assert!(orow(&s, 0).can_rerun);
    let checking = CompareState {
        checks: CheckQueue::queued(&[0]),
        ..s
    };
    assert!(!orow(&checking, 0).can_rerun);
    assert!(!checking.can_rerun(0));
}

#[test]
fn open_needs_the_results_to_be_held() {
    let s = ran(state());
    let s = CompareState {
        results: [(
            10,
            crate::viewmodel::compare::ResultsView::new(10, &summary()),
        )]
        .into_iter()
        .collect(),
        ..s
    };
    assert!(orow(&s, 0).can_open);
}

#[test]
fn a_running_row_shows_progress_and_offers_nothing() {
    let s = ran(state());
    let r = orow(&s, 1);
    assert_eq!(r.status_key, "compare.overview.status.running");
    assert_eq!(
        r.detail,
        RowDetail::Progress {
            scanned_source: 5,
            scanned_target: 6,
            differences: 1
        }
    );
    assert!(!r.can_open && !r.can_discard && !r.can_rerun);
    assert!(!r.offers_duplicates && !r.can_find_duplicates);
}

#[test]
fn a_failed_row_carries_its_error_and_only_a_duplicate_key_failure_offers_the_lookup() {
    let s = ran(state());
    let overview = s
        .overview
        .clone()
        .pair_done(1, 11, PairOutcome::Failed(CoreError::CompareKeyMissing))
        .run_done(1);
    let plain = CompareState {
        overview,
        ..s.clone()
    };
    let r = orow(&plain, 1);
    assert_eq!(r.detail, RowDetail::Error(CoreError::CompareKeyMissing));
    assert!(!r.offers_duplicates && r.can_discard && r.can_rerun);

    let overview = s
        .overview
        .clone()
        .pair_done(1, 11, PairOutcome::Failed(CoreError::CompareDuplicateKey))
        .run_done(1);
    // The lookup also needs a fresh passing verdict for the entry.
    let dup = checked(
        CompareState {
            overview,
            ..s.clone()
        },
        1,
        pre(false, 5),
    );
    let found = orow(&dup, 1);
    assert!(found.offers_duplicates && found.can_find_duplicates);
    // Without a fresh passing verdict the button is shown but not live.
    let unchecked = orow(&plain_dup(&s), 1);
    assert!(unchecked.offers_duplicates && !unchecked.can_find_duplicates);
    // Once the lookup failed, the panel's Try again replaces the row's button.
    let failed = CompareState {
        duplicates: crate::viewmodel::compare::DuplicatesView::Failed { entry: 1 },
        ..dup
    };
    assert!(!orow(&failed, 1).offers_duplicates);
}

fn plain_dup(s: &CompareState) -> CompareState {
    let overview = s
        .overview
        .clone()
        .pair_done(1, 11, PairOutcome::Failed(CoreError::CompareDuplicateKey))
        .run_done(1);
    CompareState {
        overview,
        ..s.clone()
    }
}

#[test]
fn a_discarded_row_has_no_detail_and_can_only_be_rerun() {
    let s = ran(state());
    let s = CompareState {
        overview: s.overview.clone().run_done(1).discarded(10),
        ..s
    };
    let r = orow(&s, 0);
    assert_eq!(r.status_key, "compare.overview.status.discarded");
    assert_eq!(r.detail, RowDetail::None);
    assert!(!r.can_open && !r.can_discard && r.can_rerun);
}

#[test]
fn an_applied_completed_row_is_marked_with_a_warning_icon_and_its_own_label() {
    let s = ran(state());
    let s = CompareState {
        overview: s.overview.clone().marked_applied(10),
        ..s
    };
    let r = orow(&s, 0);
    assert_eq!(r.status_key, "compare.overview.status.applied");
    assert_eq!(r.icon, StatusIcon::Warning);
    // The counts stay visible, and a pair no apply touched keeps its own status.
    assert!(matches!(r.detail, RowDetail::Counts(_)));
    assert_eq!(orow(&s, 1).status_key, "compare.overview.status.running");
}

#[test]
fn rows_of_removed_entries_have_no_overview_row() {
    let s = ran(state());
    let s = CompareState {
        pairs: s.pairs.removed(0),
        ..s
    };
    // The row of a removed entry cannot be named; the reducer drops it with the entry.
    assert!(s.overview_row(0).is_none());
}

#[test]
fn totals_become_counts_the_totals_row_can_show() {
    let s = ran(state());
    let totals = s.overview.totals();
    assert_eq!(
        RowCounts::from(&totals),
        RowCounts {
            different: 3,
            only_in_source: 1,
            only_in_target: 2,
            identical: 7
        }
    );
}

#[test]
fn the_open_pair_is_named_by_its_entry() {
    let s = ran(state());
    assert_eq!(s.open_pair_labels(), None);
    let open = CompareState {
        open_pair: Some(11),
        ..s
    };
    assert_eq!(
        open.open_pair_labels(),
        Some(("a.orders".to_string(), "b.orders_copy".to_string()))
    );
    let gone = CompareState {
        open_pair: Some(99),
        ..open
    };
    assert_eq!(gone.open_pair_labels(), None);
}

#[test]
fn the_editor_shows_the_issue_of_the_edited_pair_only() {
    let s = state();
    assert_eq!(s.editor_issue(), None, "nothing is being edited");
    let editing = CompareState {
        pairs: s.pairs.clone().editing(Some(0)),
        ..s.clone()
    };
    assert_eq!(editing.editor_issue(), None);
    let bad = CompareState {
        setup: SetupForm::default().with_key_text("a,,b"),
        ..editing
    };
    assert_eq!(
        bad.editor_issue(),
        Some(crate::viewmodel::compare::SetupError::Key)
    );
}

#[test]
fn a_checked_row_carries_both_sides_counts_and_index_labels() {
    let s = state();
    assert_eq!(row(&s, 0).sizes, None, "nothing checked yet");
    let answer = Preflight {
        source_count: 12,
        target_count: 7,
        source_index: IndexStatus::Indexed,
        target_index: IndexStatus::NotIndexed,
        unindexed_too_large: false,
    };
    let sizes = row(&checked(s, 0, answer), 0).sizes.expect("sizes");
    assert_eq!((sizes.source_count, sizes.target_count), (12, 7));
    assert_eq!(sizes.source_index_key, "compare.preflight.index_indexed");
    assert_eq!(sizes.target_index_key, "compare.preflight.index_missing");
}

#[test]
fn a_pair_row_can_be_removed_only_while_nothing_is_active() {
    let s = state();
    assert!(row(&s, 0).can_remove);
    let running = ran(s.clone());
    assert!(!row(&running, 0).can_remove, "a run is active");
    let checking = CompareState {
        checks: CheckQueue::queued(&[0]),
        ..s
    };
    assert!(!row(&checking, 0).can_remove, "a check is queued");
}
