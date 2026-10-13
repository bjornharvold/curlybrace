//! Click-driven Compare APPLY flows: the REAL app and runtime against a real MongoDB. The
//! gestures are ticking rows, choosing an action, Review changes, typing the confirmation and
//! Apply; the assertions read both the screen and the database. Tier 1 like
//! `local_compare_flows.rs`, but these tests WRITE to the compared collections: run them against
//! a throwaway server of your own (see docs/superpowers/notes/testing.md). Only `cbit_*`
//! databases are used and the fixture drops them.
mod support;

use bson::doc;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::error::CoreError;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::viewmodel::compare::{ApplySummary, ApplyUi, PairStatus, StatusIcon};
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
        // These flows write (one inserts about 50 MB): never fall back to the default server.
        if std::env::var_os("CURLYBRACE_IT").is_some()
            && std::env::var_os("CURLYBRACE_IT_MONGODB_URI").is_none()
        {
            eprintln!(
                "SKIPPED {}: set CURLYBRACE_IT_MONGODB_URI to a throwaway server (not 27017)",
                test_name!()
            );
            return;
        }
        let Some(f) = Fixture::local(test_name!()) else {
            return;
        };
        f
    }};
}

/// The 50 MB flow loads the server for a while, which can starve the listings of the other
/// flows past their bounds: it runs alone (write lock), the others run together (read lock).
static SERVER_LOAD: std::sync::RwLock<()> = std::sync::RwLock::new(());

fn shared() -> std::sync::RwLockReadGuard<'static, ()> {
    SERVER_LOAD.read().unwrap_or_else(|e| e.into_inner())
}

fn exclusive() -> std::sync::RwLockWriteGuard<'static, ()> {
    SERVER_LOAD.write().unwrap_or_else(|e| e.into_inner())
}

fn ids_of(docs: Vec<bson::Document>) -> Vec<i32> {
    docs.iter()
        .map(|d| d.get_i32("_id").expect("integer _id"))
        .collect()
}

fn numbered(range: std::ops::Range<i32>, offset: i32) -> Vec<bson::Document> {
    range.map(|i| doc! {"_id": i, "n": i + offset}).collect()
}

/// One pair `left` -> `right` of the test database, checked and run; its results are open.
#[track_caller]
fn compared(f: &mut Fixture, limit: std::time::Duration) {
    open_compare(f);
    pick_pair_and_check(f, "left", "right");
    start_and_wait(f, limit);
    wait_shows_key(f, "compare.results.back_to_overview");
}

fn apply_summary(f: &Fixture) -> ApplySummary {
    let apply = &f.harness.state().state().compare.apply;
    apply.summary().expect("the apply finished")
}

fn marked_applied(f: &Fixture, collection: &str) -> bool {
    let entry = entry_of(f, collection);
    let overview = &f.harness.state().state().compare.overview;
    overview.row_for(entry).is_some_and(|r| r.applied)
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn copy_whole_collection_to_an_empty_target_then_rerun_shows_no_differences() {
    let _lock = shared();
    let mut f = fixture!();
    // More than one page of 100 rows: the bulk path, not an explicit key list.
    f.db.insert("left", numbered(0..250, 0));
    f.db.create_collection("right");
    compared(&mut f, SHORT);

    open_tab(&mut f, "compare.category.only_in_source", 250);
    select_all_in_tab(&mut f, 250);
    click_action(&mut f, "compare.apply.action.copy_to_target");
    open_review(&mut f);
    assert!(apply_enabled(&f), "inserts need no typed confirmation");
    confirm_and_wait(&mut f, SHORT);

    let summary = apply_summary(&f);
    assert_eq!(
        (summary.applied, summary.failed, summary.not_run),
        (250, 0, 0)
    );
    assert_eq!(f.db.count("right"), 250);
    assert_eq!(
        f.db.all("right"),
        f.db.all("left"),
        "ids and fields converge"
    );
    dismiss_result(&mut f);

    assert!(marked_applied(&f, "left"), "Applied until it is run again");
    let rerun = rerun(&mut f, "left");
    assert_eq!(
        (
            rerun.equal,
            rerun.different,
            rerun.only_in_source,
            rerun.only_in_target
        ),
        (250, 0, 0, 0)
    );
    assert!(!marked_applied(&f, "left"), "the re-run clears the marker");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn delete_extras_requires_the_typed_name_and_removes_only_selected_rows() {
    let _lock = shared();
    let mut f = fixture!();
    f.db.insert("left", numbered(1..4, 0));
    let mut right = numbered(1..4, 0);
    right.extend(numbered(10..15, 0));
    f.db.insert("right", right);
    compared(&mut f, SHORT);

    open_tab(&mut f, "compare.category.only_in_target", 5);
    tick_rows(&mut f, &["11", "13"]);
    click_action(&mut f, "compare.apply.action.delete_from_target");
    open_review(&mut f);

    // Apply stays closed until the collection name is typed exactly.
    assert!(!apply_enabled(&f), "a delete needs the typed name");
    type_confirmation(&mut f, "righ");
    assert!(!apply_enabled(&f), "a prefix is not enough");
    type_confirmation(&mut f, "Right");
    assert!(!apply_enabled(&f), "the comparison is case-sensitive");
    type_confirmation(&mut f, "right");
    assert!(apply_enabled(&f));
    assert_eq!(f.db.count("right"), 8, "nothing is written before Apply");
    confirm_and_wait(&mut f, SHORT);

    let summary = apply_summary(&f);
    assert_eq!((summary.applied, summary.failed, summary.stale), (2, 0, 0));
    assert_eq!(ids_of(f.db.all("right")), vec![1, 2, 3, 10, 12, 14]);
    assert_eq!(
        ids_of(f.db.all("left")),
        vec![1, 2, 3],
        "the source is untouched"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn replace_changed_documents_and_converge() {
    let _lock = shared();
    let mut f = fixture!();
    f.db.insert("left", numbered(0..6, 0));
    f.db.insert("right", numbered(0..6, 100));
    compared(&mut f, SHORT);

    open_tab(&mut f, "compare.category.different", 6);
    select_all_in_tab(&mut f, 6);
    click_action(&mut f, "compare.apply.action.copy_to_target");
    open_review(&mut f);
    confirm_and_wait(&mut f, SHORT);

    assert_eq!(apply_summary(&f).applied, 6);
    assert_eq!(f.db.all("right"), f.db.all("left"));
    dismiss_result(&mut f);
    let rerun = rerun(&mut f, "left");
    assert_eq!((rerun.equal, rerun.different), (6, 0));
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_document_changed_after_the_compare_is_reported_stale_and_left_alone() {
    let _lock = shared();
    let mut f = fixture!();
    f.db.insert("left", numbered(0..3, 0));
    f.db.insert("right", numbered(0..3, 100));
    compared(&mut f, SHORT);

    open_tab(&mut f, "compare.category.different", 3);
    select_all_in_tab(&mut f, 3);
    click_action(&mut f, "compare.apply.action.copy_to_target");
    open_review(&mut f);
    // A second client changes one target document between the compare and the apply.
    f.db.set_fields("right", doc! {"_id": 1_i32}, doc! {"n": 999_i32});
    confirm_and_wait(&mut f, SHORT);

    let summary = apply_summary(&f);
    assert_eq!((summary.applied, summary.stale, summary.failed), (2, 1, 0));
    let skipped = f
        .labels
        .t_with("compare.apply.outcome.stale", &[("count", "1")]);
    let skipped = format!("{} {skipped}", StatusIcon::Warning.glyph());
    wait_shows(&mut f, &skipped);
    let n_of = |f: &Fixture, id: i32| {
        let found = f.db.find_one("right", doc! {"_id": id});
        found.expect("document exists").get_i32("n").expect("n")
    };
    assert_eq!(n_of(&f, 1), 999, "the concurrent change is not overwritten");
    assert_eq!(
        (n_of(&f, 0), n_of(&f, 2)),
        (0, 2),
        "the others were replaced"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_unique_index_collision_is_listed_as_a_failure_and_neighbours_apply() {
    let _lock = shared();
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![
            doc! {"_id": 1_i32, "code": "a"},
            doc! {"_id": 2_i32, "code": "b"},
            doc! {"_id": 3_i32, "code": "c"},
        ],
    );
    // The target already holds code "a" under another _id, and forbids a second one.
    f.db.insert("right", vec![doc! {"_id": 100_i32, "code": "a"}]);
    f.db.create_unique_index("right", "code");
    compared(&mut f, SHORT);

    open_tab(&mut f, "compare.category.only_in_source", 3);
    select_all_in_tab(&mut f, 3);
    click_action(&mut f, "compare.apply.action.copy_to_target");
    open_review(&mut f);
    confirm_and_wait(&mut f, SHORT);

    let summary = apply_summary(&f);
    assert_eq!((summary.applied, summary.failed), (2, 1));
    assert_eq!(
        ids_of(f.db.all("right")),
        vec![2, 3, 100],
        "neighbours written"
    );
    wait_shows_key(&mut f, "compare.apply.finished.failures_title");
    let message = error_text(&f, &CoreError::ApplyDuplicateKey);
    wait_shows(&mut f, &message);
    assert!(
        !shows_containing(&f.harness, "E11000")
            && !shows_containing(&f.harness, "duplicate key error"),
        "no raw driver text on screen"
    );
    assert!(
        is_on_screen(&f.harness, "1"),
        "the failed document's key is listed"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn cancel_a_large_apply_reports_not_run_and_marks_the_pair_applied() {
    let _lock = exclusive();
    let mut f = fixture!();
    // About 50 MB to insert: far more than is written between the Apply and the Cancel click.
    let total: i32 = 50_000;
    f.db.insert("left", padded(0..total, 1_000));
    f.db.create_collection("right");
    compared(&mut f, LONG);

    open_tab(&mut f, "compare.category.only_in_source", total as usize);
    select_all_in_tab(&mut f, total as usize);
    click_action(&mut f, "compare.apply.action.copy_to_target");
    open_review(&mut f);
    click_key(&mut f, "compare.apply.review.apply");
    // As soon as the first progress is in (something was written), cancel it.
    wait_until(&mut f.harness, "the first progress", LONG, |h| {
        let apply = &h.state().state().compare.apply;
        let written =
            matches!(apply, ApplyUi::Running { rows, .. } if rows.iter().any(|r| r.done > 0));
        written && apply.can_cancel()
    });
    click_key(&mut f, "compare.apply.running.cancel");
    wait_until(&mut f.harness, "the apply to end", LONG, |h| {
        h.state().state().compare.apply.is_finished()
    });
    wait_shows_key(&mut f, "compare.apply.finished.cancelled_note");

    let summary = apply_summary(&f);
    assert_eq!(
        summary.total(),
        total as u64,
        "the counts add up to the request"
    );
    assert!(
        summary.not_run > 0,
        "the cancel stopped the writes: {summary:?}"
    );
    assert_eq!(summary.failed, 0);
    assert_eq!(
        f.db.count("right"),
        summary.applied,
        "applied is what was written"
    );
    assert!(
        marked_applied(&f, "left"),
        "the pair carries the Applied marker"
    );
    dismiss_result(&mut f);
    click_key(&mut f, "compare.results.back_to_overview");
    let marker = status_label(&f, StatusIcon::Warning, "compare.overview.status.applied");
    wait_shows(&mut f, &marker);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn two_pairs_in_one_apply_report_separately() {
    let _lock = shared();
    let mut f = fixture!();
    let target = Scratch::new(&f);
    // Pair "a": three documents only in the source. Pair "b": two extras only in the target.
    f.db.insert("a", numbered(0..3, 0));
    target.0.create_collection("a");
    f.db.insert("b", numbered(0..2, 0));
    let mut b = numbered(0..2, 0);
    b.extend(numbered(50..52, 0));
    target.0.insert("b", b);
    open_compare(&mut f);
    let source_db = f.db.name.clone();
    pick_database(&mut f, Side::Source, &source_db);
    pick_database(&mut f, Side::Target, &target.0.name);
    send(&mut f, CompareAction::AddAllSameNamed);
    check_all(&mut f);
    send(&mut f, CompareAction::Start);
    wait_finished(&mut f, SHORT);

    let a = entry_of(&f, "a");
    send(&mut f, CompareAction::OpenPair(a));
    open_tab(&mut f, "compare.category.only_in_source", 3);
    select_all_in_tab(&mut f, 3);
    click_action(&mut f, "compare.apply.action.copy_to_target");
    click_key(&mut f, "compare.results.back_to_overview");
    let b = entry_of(&f, "b");
    send(&mut f, CompareAction::OpenPair(b));
    open_tab(&mut f, "compare.category.only_in_target", 2);
    select_all_in_tab(&mut f, 2);
    click_action(&mut f, "compare.apply.action.delete_from_target");
    open_review(&mut f);
    // The deletes all hit one collection: its name is the text to type.
    assert!(!apply_enabled(&f));
    type_confirmation(&mut f, "b");
    confirm_and_wait(&mut f, SHORT);

    let reports = match &f.harness.state().state().compare.apply {
        ApplyUi::Finished { reports, .. } => reports.clone(),
        other => panic!("expected a finished apply, found {other:?}"),
    };
    assert_eq!(reports.len(), 2, "one report per pair");
    let mut applied: Vec<u64> = reports.iter().map(|(_, r)| r.applied).collect();
    applied.sort_unstable();
    assert_eq!(applied, vec![2, 3]);
    assert!(reports.iter().all(|(_, r)| r.failed == 0 && r.stale == 0));
    for applied in [3_u64, 2] {
        let line = f.labels.t_with(
            "compare.apply.finished.pair_counts",
            &[
                ("applied", &applied.to_string()),
                ("stale", "0"),
                ("not_planned", "0"),
                ("failed", "0"),
                ("not_run", "0"),
            ],
        );
        wait_shows(&mut f, &line);
    }
    assert_eq!(ids_of(target.0.all("a")), vec![0, 1, 2]);
    assert_eq!(ids_of(target.0.all("b")), vec![0, 1]);
    for name in ["a", "b"] {
        assert!(marked_applied(&f, name), "{name} is marked Applied");
        let entry = entry_of(&f, name);
        assert!(matches!(status_of(&f, entry), PairStatus::Completed(_)));
    }
}
