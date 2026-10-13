//! Click-driven Compare flows: the REAL app and runtime against a real MongoDB, through the
//! accessibility tree. Tier 1 like `local_e2e_flows.rs` (see its header): only throwaway `cbit_*`
//! databases, dropped by the fixture; run with `scripts/integration-tests.sh`. Both sides read
//! two collections of the one test database through the saved connection.
mod support;

use bson::doc;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::error::CoreError;
use curlybrace_ui::viewmodel::compare::RunView;
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
    let sides = f.labels.t_with(
        "compare.preflight.side_summary",
        &[
            ("side", &f.labels.t("compare.setup.source_heading")),
            ("count", "3"),
            ("index", &f.labels.t("compare.preflight.index_indexed")),
        ],
    );
    assert!(
        shows_containing(&f.harness, &sides),
        "counts and index shown"
    );

    start_and_wait(&mut f, SHORT);
    let different = tab(&f, "compare.category.different", 1);
    wait_shows(&mut f, &different);
    let only_source = tab(&f, "compare.category.only_in_source", 1);
    let only_target = tab(&f, "compare.category.only_in_target", 1);
    wait_shows(&mut f, &only_source);
    wait_shows(&mut f, &only_target);
    let finished = f.labels.t("compare.run.finished");
    wait_shows(&mut f, &finished);

    f.click_text(&different);
    wait_shows(&mut f, "2");
    f.click_text("2");
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

    click_key(&mut f, "compare.run.discard");
    wait_gone(&mut f, &different);
    wait_gone(&mut f, &title);
    assert!(f.harness.state().state().compare.results.is_none());
    assert_eq!(f.harness.state().state().compare.run, RunView::Idle);
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
    set_key(&mut f, "sku");
    check(&mut f);

    start_and_wait(&mut f, SHORT);
    let failed = f.labels.t("compare.run.failed");
    wait_shows(&mut f, &failed);
    let message = error_text(&f, &CoreError::CompareDuplicateKey);
    wait_shows(&mut f, &message);
    assert!(f.harness.state().state().compare.results.is_none());
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
    click_key(&mut f, "compare.run.start");
    wait_until(&mut f.harness, "a completed compare", SHORT, |h| {
        matches!(
            h.state().state().compare.run,
            RunView::Finished { failure: None, .. }
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

    click_key(&mut f, "compare.run.start");
    click_key(&mut f, "compare.run.cancel");
    wait_finished(&mut f, LONG);
    wait_shows_key(&mut f, "compare.run.cancelled_no_results");
    assert!(f.harness.state().state().compare.results.is_none());

    // The same pair again: Check, then Start, now left to run to its end.
    check(&mut f);
    start_and_wait(&mut f, LONG);
    wait_shows_key(&mut f, "compare.run.finished");
    wait_shows_key(&mut f, "compare.results.identical");
}
