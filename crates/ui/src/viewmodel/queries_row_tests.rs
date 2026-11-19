//! The suggested index as data and the last run on a row.
use super::*;
use crate::viewmodel::index_detail::Age;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ShapeBucket, ShapeSource, ShapeSummary};
use curlybrace_core::shape::{from_query, suggest_for_shape};
use curlybrace_core::store::{Namespace, Query};

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn shape_json(field: &str) -> String {
    from_query(&Query {
        filter: bson::doc! { field: 1 },
        ..Query::default()
    })
    .json()
}

fn bucket(start_secs: i64, executions: u64) -> ShapeBucket {
    ShapeBucket {
        start_secs,
        width_secs: 600,
        executions,
        misses: 0,
        gap_secs: 0,
    }
}

fn summary(field: &str, latest: VerdictKind, buckets: Vec<ShapeBucket>) -> ShapeSummary {
    ShapeSummary {
        hash: format!("hash-{field}"),
        shape_json: shape_json(field),
        source: ShapeSource::App,
        executions: 10,
        misses: 3,
        total_millis: 20,
        max_millis: 10,
        worst: latest,
        latest,
        buckets,
    }
}

fn state_of(shapes: Vec<ShapeSummary>) -> QueriesState {
    QueriesState::default().with_shapes(shapes)
}

#[test]
fn a_miss_row_carries_the_suggested_index_and_its_command() {
    let miss = summary("email", VerdictKind::CollectionScan, Vec::new());
    let rows = rows(&state_of(vec![miss.clone()]), &ns());
    let shape = Shape::from_json(&miss.shape_json).unwrap();
    let expected = suggest_for_shape(&shape).unwrap();
    assert_eq!(rows[0].hash, "hash-email");
    assert_eq!(rows[0].suggested.as_ref(), Some(&expected));
    assert_eq!(rows[0].suggestion, Some(expected.command(&ns())));
}

#[test]
fn a_good_row_has_no_suggested_index() {
    let good = summary("email", VerdictKind::IndexUsed, Vec::new());
    let rows = rows(&state_of(vec![good]), &ns());
    assert_eq!(rows[0].suggested, None);
    assert_eq!(rows[0].suggestion, None);
}

#[test]
fn last_run_is_the_newest_bucket_that_ran() {
    let s = summary(
        "email",
        VerdictKind::CollectionScan,
        vec![bucket(1_000, 2), bucket(3_000, 1), bucket(2_000, 4)],
    );
    // The end of the newest bucket (start 3_000, width 600): the run happened at or before it.
    assert_eq!(last_run_secs(&s), Some(3_600));
    assert_eq!(
        rows(&state_of(vec![s]), &ns())[0].last_run_secs,
        Some(3_600)
    );
}

#[test]
fn a_last_run_never_reads_as_after_now() {
    // The bucket still open ends after now: the run is "just now", never in the future.
    assert_eq!(last_run_age(3_600, 3_000), Age::Now);
    // A finished bucket ages from its end, which errs toward recent.
    assert_eq!(last_run_age(3_600, 3_600 + 1_200), Age::Minutes(20));
}

#[test]
fn a_wide_bucket_reads_from_its_end_not_its_start() {
    // A day bucket that started 30 hours ago ended 6 hours ago: "6 h", not "1 d".
    let day = ShapeBucket {
        start_secs: 0,
        width_secs: 86_400,
        executions: 1,
        misses: 0,
        gap_secs: 0,
    };
    let s = summary("email", VerdictKind::IndexUsed, vec![day]);
    let end = last_run_secs(&s).unwrap();
    assert_eq!(last_run_age(end, 30 * 3_600), Age::Hours(6));
}

#[test]
fn last_run_ignores_empty_buckets() {
    let s = summary(
        "email",
        VerdictKind::CollectionScan,
        vec![bucket(1_000, 2), bucket(5_000, 0)],
    );
    assert_eq!(last_run_secs(&s), Some(1_600));
}

#[test]
fn no_bucket_ran_means_no_last_run() {
    let none = summary("email", VerdictKind::CollectionScan, Vec::new());
    assert_eq!(last_run_secs(&none), None);
    let empty = summary("email", VerdictKind::CollectionScan, vec![bucket(1_000, 0)]);
    assert_eq!(last_run_secs(&empty), None);
}

#[test]
fn row_order_is_unchanged() {
    let state = state_of(vec![
        summary("good", VerdictKind::IndexUsed, Vec::new()),
        summary("sort", VerdictKind::SortNotCovered, Vec::new()),
        summary("scan", VerdictKind::CollectionScan, Vec::new()),
    ]);
    let order: Vec<String> = rows(&state, &ns()).into_iter().map(|r| r.hash).collect();
    assert_eq!(order, ["hash-scan", "hash-sort", "hash-good"]);
}
