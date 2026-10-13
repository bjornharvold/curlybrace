//! Click-driven Compare flows: the REAL app and runtime against a real MongoDB, through the
//! accessibility tree. Tier 1 like `local_e2e_flows.rs` (see its header): only throwaway `cbit_*`
//! databases, dropped by the fixture; run with `scripts/integration-tests.sh`. Both sides read
//! two collections of the one test database through the saved connection.
mod support;

use bson::doc;
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::error::CoreError;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::viewmodel::compare::{
    DuplicatesView, EntryId, PairOverrides, PairStatus, StatusIcon,
};
use support::actions::*;
use support::compare::*;
use support::fixture::Fixture;
use support::screen::shows_containing;
use support::wait::{SHORT, wait_until};

/// The name of the enclosing test function (for the printed skip reason).
macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

macro_rules! fixture {
    () => {{
        let Some(f) = Fixture::local(test_name!()) else {
            return;
        };
        f
    }};
}

fn tab(f: &Fixture, key: &str, count: usize) -> String {
    let name = f.labels.t(key);
    f.labels.t_with(
        "compare.results.tab",
        &[("name", &name), ("count", &count.to_string())],
    )
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn check_start_browse_a_difference_and_discard() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![
            doc! {"_id": 1_i32, "name": "Ada", "seats": 1_i32},
            doc! {"_id": 2_i32, "name": "Grace", "seats": 2_i32},
            doc! {"_id": 3_i32, "name": "Alan", "seats": 3_i32},
        ],
    );
    f.db.insert(
        "right",
        vec![
            doc! {"_id": 1_i32, "name": "Ada", "seats": 1_i32},
            doc! {"_id": 2_i32, "name": "Grace Hopper", "seats": 2_i32},
            doc! {"_id": 4_i32, "name": "Linus", "seats": 4_i32},
        ],
    );
    open_compare(&mut f);
    pick_pair_and_check(&mut f, "left", "right");
    start_and_wait(&mut f, SHORT);
    let different = tab(&f, "compare.category.different", 1);
    wait_shows(&mut f, &different);
    let only_source = tab(&f, "compare.category.only_in_source", 1);
    let only_target = tab(&f, "compare.category.only_in_target", 1);
    wait_shows(&mut f, &only_source);
    wait_shows(&mut f, &only_target);
    // The only pair finished, so its results are open.
    wait_shows_key(&mut f, "compare.results.back_to_overview");

    f.click_text(&different);
    wait_shows(&mut f, "2");
    open_row(&mut f, "2");
    let title = f.labels.t_with("compare.detail.title", &[("key", "2")]);
    wait_shows(&mut f, &title);
    wait_shows_key(&mut f, "compare.detail.changed");
    wait_shows(&mut f, "name");
    wait_shows(&mut f, "\"Grace Hopper\"");
    wait_until(
        &mut f.harness,
        "canonical JSON of both documents",
        SHORT,
        |h| shows_containing(h, "\"$numberInt\": \"2\"") && shows_containing(h, "Grace Hopper"),
    );
    assert_eq!(f.db.count("left"), 3, "nothing is ever written");
    assert_eq!(f.db.count("right"), 3);

    click_key(&mut f, "compare.results.back_to_overview");
    click_key(&mut f, "compare.overview.discard");
    wait_gone(&mut f, &different);
    wait_gone(&mut f, &title);
    assert!(f.harness.state().state().compare.results.is_empty());
    assert_eq!(first_status(&f), Some(PairStatus::Discarded));
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_duplicate_key_is_reported_and_leaves_no_partial_results() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![
            doc! {"_id": 1_i32, "sku": "a"},
            doc! {"_id": 2_i32, "sku": "a"},
            doc! {"_id": 3_i32, "sku": "b"},
        ],
    );
    f.db.insert("right", vec![doc! {"_id": 1_i32, "sku": "a"}]);
    open_compare(&mut f);
    pick_collection(&mut f, Side::Source, "left");
    pick_collection(&mut f, Side::Target, "right");
    add_pair(&mut f);
    set_key(&mut f, "sku");
    check(&mut f);

    start_and_wait(&mut f, SHORT);
    let failed = status_label(&f, StatusIcon::Failed, "compare.overview.status.failed");
    wait_shows(&mut f, &failed);
    let message = error_text(&f, &CoreError::CompareDuplicateKey);
    wait_shows(&mut f, &message);
    assert!(f.harness.state().state().compare.results.is_empty());
    assert!(
        !shows_containing(&f.harness, &f.labels.t("compare.category.different")),
        "no result tabs after a failed run"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn finding_duplicate_keys_lists_the_colliding_values() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![
            doc! {"_id": 1_i32, "email": "dup@example.com"},
            doc! {"_id": 2_i32, "email": "dup@example.com"},
            doc! {"_id": 3_i32, "email": "solo@example.com"},
        ],
    );
    f.db.insert(
        "right",
        vec![doc! {"_id": 1_i32, "email": "dup@example.com"}],
    );
    open_compare(&mut f);
    pick_collection(&mut f, Side::Source, "left");
    pick_collection(&mut f, Side::Target, "right");
    add_pair(&mut f);
    set_key(&mut f, "email");
    check(&mut f);
    start_and_wait(&mut f, SHORT);
    let message = error_text(&f, &CoreError::CompareDuplicateKey);
    wait_shows(&mut f, &message);

    click_key(&mut f, "compare.duplicates.find");
    let row = f.labels.t_with(
        "compare.duplicates.row",
        &[("key", "\"dup@example.com\""), ("count", "2")],
    );
    wait_shows(&mut f, &row);
    wait_shows_key(&mut f, "compare.duplicates.none_on_side");
    assert!(shows_containing(
        &f.harness,
        &f.labels.t("compare.duplicates.hint")
    ));
    assert!(
        !shows_containing(&f.harness, "solo@example.com"),
        "only repeated values are listed"
    );

    // The hint's next step: a unique key, checked again, then the compare completes.
    set_key(&mut f, "_id");
    check(&mut f);
    click_key(&mut f, "compare.preflight.run");
    wait_until(&mut f.harness, "a completed compare", SHORT, |h| {
        let overview = &h.state().state().compare.overview;
        overview.active.is_none()
            && matches!(
                overview.rows.first().map(|r| &r.status),
                Some(PairStatus::Completed(_))
            )
    });
    assert_eq!(f.db.count("left"), 3, "nothing is ever written");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn cancelling_a_large_run_leaves_the_workspace_usable() {
    let mut f = fixture!();
    // About 30 MB per side: far too much to scan between the Start click and the Cancel click.
    f.db.insert("left", padded(0..30_000, 1_000));
    f.db.insert("right", padded(0..30_000, 1_000));
    open_compare(&mut f);
    pick_pair_and_check(&mut f, "left", "right");

    click_key(&mut f, "compare.preflight.run");
    click_key(&mut f, "compare.overview.cancel");
    wait_finished(&mut f, LONG);
    let cancelled = status_label(
        &f,
        StatusIcon::Cancelled,
        "compare.overview.status.cancelled",
    );
    wait_shows(&mut f, &cancelled);
    assert!(f.harness.state().state().compare.results.is_empty());

    // The same pair again: Check, then Start, now left to run to its end.
    check(&mut f);
    start_and_wait(&mut f, LONG);
    wait_shows_key(&mut f, "compare.results.back_to_overview");
    wait_shows_key(&mut f, "compare.results.identical");
}

/// Source database: one collection per outcome (identical, different, source-only, duplicate
/// key) against a target that lacks `only_src`.
fn seed_two_databases(f: &Fixture, target: &Scratch) {
    let same: Vec<_> = (0..40_i32).map(|i| doc! {"_id": i, "n": i}).collect();
    let changed = |n: i32| (0..150_i32).map(move |i| doc! {"_id": i, "n": i + n});
    f.db.insert("same_a", same.clone());
    target.0.insert("same_a", same);
    f.db.insert("same_b", changed(0).collect());
    target.0.insert("same_b", changed(1).collect());
    f.db.insert("only_src", (0..7_i32).map(|i| doc! {"_id": i}).collect());
    let dup = |sku: &str, id: i32| doc! {"_id": id, "sku": sku};
    f.db.insert("dup", vec![dup("a", 1), dup("a", 2), dup("b", 3)]);
    target.0.insert("dup", vec![dup("a", 1)]);
}

/// Add all same-named, the one with no counterpart, its own key for `dup`, then Check and Run.
#[track_caller]
fn build_pairs_and_run(f: &mut Fixture, target: &Scratch) {
    open_compare(f);
    let source_db = f.db.name.clone();
    pick_database(f, Side::Source, &source_db);
    pick_database(f, Side::Target, &target.0.name);
    send(f, CompareAction::AddAllSameNamed);
    let (side, name) = (Side::Source, "only_src".to_string());
    send(
        f,
        CompareAction::AddUnmatched {
            side,
            as_name: name.clone(),
            name,
        },
    );
    assert_eq!(f.harness.state().state().compare.pairs.entries.len(), 4);
    let dup = entry_of(f, "dup");
    let own_key = PairOverrides::default().with_key_text(Some("sku".into()));
    send(f, CompareAction::SetPairOverrides(dup, own_key));
    check_all(f);
    send(f, CompareAction::Start);
    wait_finished(f, SHORT);
}

fn completed(f: &Fixture, collection: &str) -> Summary {
    match status_of(f, entry_of(f, collection)) {
        PairStatus::Completed(summary) => summary,
        other => panic!("{collection} should have completed, found {other:?}"),
    }
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_database_level_compare_runs_every_pair_and_recovers_from_a_duplicate_key() {
    let mut f = fixture!();
    let target = Scratch::new(&f);
    seed_two_databases(&f, &target);
    build_pairs_and_run(&mut f, &target);

    let (a, b, only) = (
        completed(&f, "same_a"),
        completed(&f, "same_b"),
        completed(&f, "only_src"),
    );
    assert_eq!((a.equal, a.different, a.only_in_source), (40, 0, 0));
    assert_eq!((b.different, b.equal), (150, 0));
    assert_eq!((only.only_in_source, only.only_in_target), (7, 0));
    let dup = entry_of(&f, "dup");
    let failed = PairStatus::Failed(CoreError::CompareDuplicateKey);
    assert_eq!(status_of(&f, dup), failed);
    let compare = &f.harness.state().state().compare;
    assert!(
        compare.open_pair.is_none(),
        "several pairs: the overview stays"
    );
    assert_eq!(compare.overview.totals().completed, 3);

    browse_two_pages(&mut f);
    find_then_rerun_with_a_unique_key(&mut f, dup);
}

/// Opens `same_b` and pages its 150 differing rows (100, then 50).
#[track_caller]
fn browse_two_pages(f: &mut Fixture) {
    let entry = entry_of(f, "same_b");
    send(f, CompareAction::OpenPair(entry));
    wait_page(f, 0, 100);
    send(f, CompareAction::Page(1));
    wait_page(f, 100, 50);
    send(f, CompareAction::BackToOverview);
    assert!(f.harness.state().state().compare.open_pair.is_none());
}

#[track_caller]
fn wait_page(f: &mut Fixture, offset: usize, rows: usize) {
    wait_until(&mut f.harness, "the page of rows", SHORT, |h| {
        let results = h.state().state().compare.open_results().cloned();
        results.is_some_and(|r| r.offset == offset && r.rows.len() == rows && !r.is_loading())
    });
}

/// Find duplicate keys on the failed row, then Re-run it keyed on `_id`.
#[track_caller]
fn find_then_rerun_with_a_unique_key(f: &mut Fixture, dup: EntryId) {
    send(f, CompareAction::FindDuplicates(dup));
    wait_until(&mut f.harness, "the duplicate keys", SHORT, |h| {
        matches!(
            h.state().state().compare.duplicates,
            DuplicatesView::Loaded { .. }
        )
    });
    match &f.harness.state().state().compare.duplicates {
        DuplicatesView::Loaded {
            entry,
            source,
            target,
        } => {
            assert_eq!(*entry, dup);
            assert_eq!(source.groups.len(), 1, "only \"a\" repeats");
            assert_eq!(source.groups[0].count, 2);
            assert!(target.groups.is_empty());
        }
        other => panic!("expected the loaded panel, found {other:?}"),
    }
    send(f, CompareAction::Rerun(dup));
    let unique = PairOverrides::default().with_key_text(Some("_id".into()));
    send(f, CompareAction::SetPairOverrides(dup, unique));
    check_all(f);
    send(f, CompareAction::Start);
    wait_finished(f, SHORT);
    let rerun = completed(f, "dup");
    assert_eq!((rerun.equal, rerun.only_in_source), (1, 2));
    assert_eq!(
        completed(f, "same_a").equal,
        40,
        "other rows keep their results"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn cancelling_a_multi_pair_run_ends_every_row_once() {
    let mut f = fixture!();
    let target = Scratch::new(&f);
    seed_two_databases(&f, &target);
    // About 30 MB per side: far more than is scanned between Start and Cancel.
    f.db.insert("big", padded(0..30_000, 1_000));
    target.0.insert("big", padded(0..30_000, 1_000));
    open_compare(&mut f);
    let source_db = f.db.name.clone();
    pick_database(&mut f, Side::Source, &source_db);
    pick_database(&mut f, Side::Target, &target.0.name);
    send(&mut f, CompareAction::AddAllSameNamed);
    let dup = entry_of(&f, "dup");
    let unique_key = PairOverrides::default().with_key_text(Some("_id".into()));
    send(&mut f, CompareAction::SetPairOverrides(dup, unique_key));
    check_all(&mut f);

    send(&mut f, CompareAction::Start);
    send(&mut f, CompareAction::Cancel);
    wait_finished(&mut f, LONG);
    let overview = f.harness.state().state().compare.overview.clone();
    assert_eq!(overview.rows.len(), 4);
    for row in &overview.rows {
        assert!(row.status.is_settled(), "{:?} is not terminal", row.status);
    }
    let totals = overview.totals();
    assert_eq!(totals.completed + totals.failed + totals.cancelled, 4);
    assert!(
        totals.cancelled >= 1,
        "the large pair was cancelled: {totals:?}"
    );
    assert!(overview.active.is_none(), "RunDone ended the run");
    assert!(!f.harness.state().state().compare.is_running());
}
