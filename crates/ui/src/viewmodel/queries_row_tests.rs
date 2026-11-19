//! The suggested index as data and the last run on a row.
use super::*;
use crate::viewmodel::index_detail::Age;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ShapeBucket, ShapeSource, ShapeSummary};
use curlybrace_core::shape::{from_query, suggest_for_shape};
use curlybrace_core::store::{Namespace, Query};

const TEN_MIN: i64 = 600;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

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
    wide(start_secs, TEN_MIN, executions)
}

fn wide(start_secs: i64, width_secs: i64, executions: u64) -> ShapeBucket {
    ShapeBucket {
        start_secs,
        width_secs,
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

fn scan_with(buckets: Vec<ShapeBucket>) -> ShapeSummary {
    summary("email", VerdictKind::CollectionScan, buckets)
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
    assert_eq!(suggested_of(&miss), Some(expected));
}

#[test]
fn a_good_row_has_no_suggested_index() {
    let good = summary("email", VerdictKind::IndexUsed, Vec::new());
    assert_eq!(suggested_of(&good), None);
    let rows = rows(&state_of(vec![good]), &ns());
    assert_eq!(rows[0].suggested, None);
    assert_eq!(rows[0].suggestion, None);
}

#[test]
fn last_run_is_the_newest_bucket_that_ran_with_its_end_and_width() {
    let s = scan_with(vec![bucket(1_000, 2), bucket(3_000, 1), bucket(2_000, 4)]);
    let expected = LastRunBucket {
        end_secs: 3_600,
        width_secs: TEN_MIN,
    };
    assert_eq!(last_run_bucket(&s), Some(expected));
    assert_eq!(rows(&state_of(vec![s]), &ns())[0].last_run, Some(expected));
}

#[test]
fn last_run_ignores_empty_buckets() {
    let s = scan_with(vec![bucket(1_000, 2), bucket(5_000, 0)]);
    assert_eq!(last_run_bucket(&s).map(|b| b.end_secs), Some(1_600));
}

#[test]
fn no_bucket_ran_means_no_last_run() {
    assert_eq!(last_run_bucket(&scan_with(Vec::new())), None);
    assert_eq!(last_run_bucket(&scan_with(vec![bucket(1_000, 0)])), None);
}

fn at(width_secs: i64, end_secs: i64) -> LastRunBucket {
    LastRunBucket {
        end_secs,
        width_secs,
    }
}

#[test]
fn the_ten_minute_tier_still_reads_minutes() {
    // Ended 20 minutes ago.
    assert_eq!(
        last_run(at(TEN_MIN, 10_000), 10_000 + 1_200),
        LastRun::Ago(Age::Minutes(20))
    );
    // Ended 3 hours ago.
    assert_eq!(
        last_run(at(TEN_MIN, 10_000), 10_000 + 3 * HOUR),
        LastRun::Ago(Age::Hours(3))
    );
}

#[test]
fn a_shape_in_an_open_bucket_is_within_that_bucket_never_just_now() {
    // The bucket ends after now: the run was somewhere in it.
    assert_eq!(
        last_run(at(TEN_MIN, 10_300), 10_000),
        LastRun::Within(BucketWidth::TenMinutes)
    );
    assert_eq!(
        last_run(at(HOUR, 13_000), 10_000),
        LastRun::Within(BucketWidth::Hour)
    );
    assert_eq!(
        last_run(at(DAY, 90_000), 10_000),
        LastRun::Within(BucketWidth::Day)
    );
}

#[test]
fn a_bucket_that_just_ended_is_still_within_its_width_not_just_now() {
    // Ended 30 seconds ago: the run could be a whole bucket older than that.
    assert_eq!(
        last_run(at(HOUR, 10_000), 10_030),
        LastRun::Within(BucketWidth::Hour)
    );
    assert_eq!(
        last_run(at(DAY, 10_000), 10_030),
        LastRun::Within(BucketWidth::Day)
    );
}

#[test]
fn no_bucket_width_ever_reads_just_now() {
    for width in [TEN_MIN, HOUR, DAY] {
        for ahead in [-DAY, -HOUR, -TEN_MIN, -59, -1, 0, 1, 59, 60, HOUR, DAY] {
            let reading = last_run(at(width, 100_000 + ahead), 100_000);
            assert_ne!(
                reading,
                LastRun::Ago(Age::Now),
                "width {width}, end {ahead} from now"
            );
        }
    }
}

#[test]
fn widths_between_the_tiers_round_up_to_the_next_one() {
    assert_eq!(BucketWidth::of(1), BucketWidth::TenMinutes);
    assert_eq!(BucketWidth::of(TEN_MIN), BucketWidth::TenMinutes);
    assert_eq!(BucketWidth::of(TEN_MIN + 1), BucketWidth::Hour);
    assert_eq!(BucketWidth::of(HOUR), BucketWidth::Hour);
    assert_eq!(BucketWidth::of(HOUR + 1), BucketWidth::Day);
    assert_eq!(BucketWidth::of(DAY * 2), BucketWidth::Day);
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
