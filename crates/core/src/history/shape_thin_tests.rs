//! Rolling, expiring and capping shape history.

use super::db::HistoryDb;
use super::db::SeriesKey;
use super::retention::{ChartRange, DAILY_RETENTION_SECS, DAY_SECS, HOUR_SECS};
use super::series::{DEFAULT_MAX_CONTINUOUS_SECS, Sample};
use super::shapes::ShapeSource;
use super::shapes_tests::{NOW, conn_id, count, key, obs};
use crate::explain::VerdictKind;

type Row = (i64, i64, i64, i64, i64, i64, String, i64);

fn buckets(db: &HistoryDb, tier: i64) -> Vec<Row> {
    db.conn
        .lock()
        .unwrap()
        .prepare(
            "SELECT tier, start_secs, executions, misses, total_millis, max_millis, worst, gap_secs
             FROM shape_bucket WHERE tier = ?1 ORDER BY start_secs",
        )
        .unwrap()
        .query_map([tier], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn thin(db: &HistoryDb) {
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
}

#[test]
fn thin_rolls_raw_into_hourly_and_daily_preserving_sums_max_and_worst() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = || key(1, ShapeSource::App, "h");
    // Three raw buckets of one hour, 10 days back (hourly after thinning).
    let hour = (NOW - 10 * DAY_SECS).div_euclid(HOUR_SECS) * HOUR_SECS;
    // Two hours of one day, 100 days back (daily after thinning).
    let day = (NOW - 100 * DAY_SECS).div_euclid(DAY_SECS) * DAY_SECS;
    db.record_shapes(&[
        obs(k(), hour, 1, 0, (10, 10), VerdictKind::IndexUsed),
        obs(
            k(),
            hour + 600,
            2,
            2,
            (200, 150),
            VerdictKind::CollectionScan,
        ),
        obs(
            k(),
            hour + 1_200,
            4,
            1,
            (40, 30),
            VerdictKind::InefficientIndex,
        ),
        obs(
            k(),
            day + 3_600,
            8,
            0,
            (80, 80),
            VerdictKind::CoveredByIndex,
        ),
        obs(
            k(),
            day + 7_200,
            16,
            4,
            (160, 20),
            VerdictKind::SortNotCovered,
        ),
    ])
    .unwrap();
    db.record_gap(conn_id(1), "node-a:27017", "shop", hour, 30)
        .unwrap();
    db.record_gap(conn_id(1), "node-a:27017", "shop", hour + 600, 40)
        .unwrap();
    thin(&db);

    assert!(buckets(&db, 0).is_empty());
    assert_eq!(
        buckets(&db, 1),
        vec![(1, hour, 7, 3, 250, 150, "collection_scan".to_owned(), 0)]
    );
    assert_eq!(
        buckets(&db, 2),
        vec![(2, day, 24, 4, 240, 80, "sort_not_covered".to_owned(), 0)]
    );
    // Gaps rolled with them.
    let gap: i64 = count(&db, "SELECT gap_secs FROM shape_gap WHERE tier = 1");
    assert_eq!(gap, 70);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_gap"), 1);
    // And the read side sees the hourly totals.
    let got = db
        .shapes_for(conn_id(1), "shop", "orders", ChartRange::Month, NOW)
        .unwrap();
    assert_eq!(got[0].executions, 7);
    assert_eq!(got[0].buckets[0].gap_secs, 70);
    // Thinning again changes nothing.
    thin(&db);
    assert_eq!(buckets(&db, 1).len(), 1);
}

#[test]
fn rolling_adds_to_an_existing_rolled_row() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = || key(1, ShapeSource::App, "h");
    let hour = (NOW - 10 * DAY_SECS).div_euclid(HOUR_SECS) * HOUR_SECS;
    db.record_shapes(&[obs(k(), hour, 1, 1, (5, 5), VerdictKind::CollectionScan)])
        .unwrap();
    thin(&db);
    db.record_shapes(&[obs(k(), hour + 600, 2, 0, (7, 7), VerdictKind::NoFilter)])
        .unwrap();
    thin(&db);
    assert_eq!(
        buckets(&db, 1),
        vec![(1, hour, 3, 1, 12, 7, "collection_scan".to_owned(), 0)]
    );
}

#[test]
fn recent_rows_are_not_rolled() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_shapes(&[obs(
        key(1, ShapeSource::App, "h"),
        NOW - 3_600,
        1,
        0,
        (1, 1),
        VerdictKind::IndexUsed,
    )])
    .unwrap();
    thin(&db);
    assert_eq!(buckets(&db, 0).len(), 1);
}

#[test]
fn retention_expires_old_rows() {
    let db = HistoryDb::open_in_memory().unwrap();
    let old = NOW - DAILY_RETENTION_SECS - 5 * DAY_SECS;
    db.record_shapes(&[
        obs(
            key(1, ShapeSource::App, "old"),
            old,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ),
        obs(
            key(1, ShapeSource::App, "new"),
            NOW - 600,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ),
    ])
    .unwrap();
    db.record_gap(conn_id(1), "h", "shop", old, 60).unwrap();
    thin(&db);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_bucket"), 1);
    // The shape with nothing left is forgotten.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 1);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM shape WHERE hash = 'new'"),
        1
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_gap"), 0);
}

#[test]
fn a_row_stamped_in_the_future_is_not_expired_by_age() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_shapes(&[obs(
        key(1, ShapeSource::App, "h"),
        NOW + 30 * DAY_SECS,
        1,
        0,
        (1, 1),
        VerdictKind::IndexUsed,
    )])
    .unwrap();
    thin(&db);
    assert_eq!(buckets(&db, 0).len(), 1);
}

#[test]
fn record_gap_rolls_and_is_purged_with_the_connection() {
    let db = HistoryDb::open_in_memory().unwrap();
    let old = (NOW - 10 * DAY_SECS).div_euclid(HOUR_SECS) * HOUR_SECS;
    db.record_gap(conn_id(1), "a", "shop", old, 10).unwrap();
    db.record_gap(conn_id(1), "a", "shop", old + 600, 20)
        .unwrap();
    db.record_gap(conn_id(1), "b", "shop", old, 5).unwrap();
    db.record_gap(conn_id(2), "a", "shop", old, 99).unwrap();
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM shape_gap WHERE tier = 0"),
        4
    );
    thin(&db);
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM shape_gap WHERE tier = 0"),
        0
    );
    assert_eq!(
        count(&db, "SELECT SUM(gap_secs) FROM shape_gap WHERE tier = 1"),
        134
    );
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM shape_gap WHERE tier = 1"),
        3
    );
    db.purge_connection(conn_id(1)).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_gap"), 1);
}

fn usage_key(n: usize) -> SeriesKey {
    SeriesKey {
        connection: conn_id(1),
        host: "h".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: format!("idx{}", n % 5),
        definition_hash: "x".into(),
    }
}

#[test]
fn cap_thins_oldest_first_across_usage_and_shapes() {
    let db = HistoryDb::open_in_memory().unwrap();
    let samples = 2_000_i64;
    let first_sample = NOW - 60 * samples;
    let batch: Vec<_> = (0..samples)
        .map(|i| {
            (
                usage_key(usize::try_from(i % 5).unwrap()),
                Sample {
                    at_secs: first_sample + 60 * i,
                    ops: i,
                    since_ms: 1_000,
                },
            )
        })
        .collect();
    db.record_many(&batch).unwrap();
    // 8 shapes x 270 raw buckets (45 hours): all inside the raw window, so nothing is rolled.
    let first_bucket = (NOW - 270 * 600).div_euclid(600) * 600;
    let rows: Vec<_> = (0..8)
        .flat_map(|s| {
            (0..270).map(move |b| {
                obs(
                    key(1, ShapeSource::App, &format!("shape-{s}")),
                    first_bucket + 600 * b,
                    3,
                    1,
                    (30, 20),
                    VerdictKind::CollectionScan,
                )
            })
        })
        .collect();
    db.record_shapes(&rows).unwrap();
    let samples_before = count(&db, "SELECT COUNT(*) FROM samples");
    let buckets_before = count(&db, "SELECT COUNT(*) FROM shape_bucket");

    let cap = db.used_bytes().unwrap() * 2 / 3;
    let db = db.with_max_bytes(cap);
    thin(&db);
    assert!(
        db.used_bytes().unwrap() <= cap,
        "the file ends under the cap"
    );

    let samples_after = count(&db, "SELECT COUNT(*) FROM samples");
    let buckets_after = count(&db, "SELECT COUNT(*) FROM shape_bucket");
    assert!(
        samples_after > 0 && buckets_after > 0,
        "neither set is emptied"
    );
    assert!(samples_after < samples_before, "usage lost its oldest rows");
    assert!(
        buckets_after < buckets_before,
        "shapes lost their oldest rows"
    );
    // Oldest first, in each set: what is left starts later than what was there.
    let oldest_sample = count(&db, "SELECT MIN(at_secs) FROM samples");
    let oldest_bucket = count(&db, "SELECT MIN(start_secs) FROM shape_bucket");
    assert!(oldest_sample > first_sample);
    assert!(oldest_bucket > first_bucket);
    // Both sets gave up a comparable share (alternating batches), neither all of its budget.
    let usage_lost = samples_before - samples_after;
    let shapes_lost = buckets_before - buckets_after;
    assert!(
        usage_lost >= 500 && shapes_lost >= 500,
        "{usage_lost} {shapes_lost}"
    );
}

#[test]
fn cap_drops_future_only_shape_rows_first() {
    let db = HistoryDb::open_in_memory().unwrap();
    let future = NOW + 100 * DAY_SECS;
    let mut rows = Vec::new();
    for b in 0..1_500 {
        rows.push(obs(
            key(1, ShapeSource::App, "future"),
            future + 600 * b,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ));
        rows.push(obs(
            key(1, ShapeSource::App, "real"),
            NOW - 600 * (b % 200) - 600,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ));
    }
    db.record_shapes(&rows).unwrap();
    let real_before = count(
        &db,
        "SELECT COUNT(*) FROM shape_bucket b JOIN shape s ON s.id = b.shape_id WHERE s.hash = 'real'",
    );
    let db = db.with_max_bytes_from_used(10_000);
    thin(&db);
    let real_after = count(
        &db,
        "SELECT COUNT(*) FROM shape_bucket b JOIN shape s ON s.id = b.shape_id WHERE s.hash = 'real'",
    );
    assert_eq!(real_after, real_before);
    assert!(
        count(
            &db,
            "SELECT COUNT(*) FROM shape_bucket b JOIN shape s ON s.id = b.shape_id WHERE s.hash = 'future'"
        ) < 1_500
    );
}

fn fill_usage(db: &HistoryDb, count: i64, first_at: i64) {
    let batch: Vec<_> = (0..count)
        .map(|i| {
            (
                usage_key(usize::try_from(i % 5).unwrap()),
                Sample {
                    at_secs: first_at + 60 * i,
                    ops: i,
                    since_ms: 1_000,
                },
            )
        })
        .collect();
    db.record_many(&batch).unwrap();
}

fn fill_shapes(db: &HistoryDb, shapes: i64, buckets: i64, first_at: i64) {
    let rows: Vec<_> = (0..shapes)
        .flat_map(|s| {
            (0..buckets).map(move |b| {
                let k = key(1, ShapeSource::App, &format!("shape-{s}"));
                obs(
                    k,
                    first_at + 600 * b,
                    3,
                    1,
                    (30, 20),
                    VerdictKind::CollectionScan,
                )
            })
        })
        .collect();
    db.record_shapes(&rows).unwrap();
}

#[test]
fn saturated_counters_do_not_break_thinning_or_the_cap() {
    use super::shape_schema::COUNT_CEILING;
    use super::shapes::ShapeObservation;
    let db = HistoryDb::open_in_memory().unwrap();
    let ceiling = u64::try_from(COUNT_CEILING).unwrap();
    let hour = (NOW - 10 * DAY_SECS).div_euclid(HOUR_SECS) * HOUR_SECS;
    let huge = |at| ShapeObservation {
        gap_secs: u64::MAX,
        ..obs(
            key(1, ShapeSource::App, "h"),
            at,
            u64::MAX,
            u64::MAX,
            (u64::MAX, u64::MAX),
            VerdictKind::CollectionScan,
        )
    };
    db.record_shapes(&[huge(hour), huge(hour), huge(hour + 600), huge(hour + 1_200)])
        .unwrap();
    db.record_shapes(&[huge(NOW - 600), huge(NOW - 600)])
        .unwrap();
    for at in [hour, hour, hour + 600, NOW - 600, NOW - 600] {
        db.record_gap(conn_id(1), "node-a:27017", "shop", at, u64::MAX)
            .unwrap();
    }
    thin(&db);
    let got = db
        .shapes_for(conn_id(1), "shop", "orders", ChartRange::Month, NOW)
        .unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].buckets.iter().all(|b| b.executions <= ceiling));
    assert!(got[0].buckets.iter().all(|b| b.gap_secs <= 86_400));
    assert!(got[0].executions >= ceiling);
    // The cap still runs for the whole file afterwards.
    fill_usage(&db, 1_500, NOW - 90_000);
    let cap = db.used_bytes().unwrap() * 3 / 4;
    let db = db.with_max_bytes(cap);
    thin(&db);
    assert!(db.used_bytes().unwrap() <= cap);
}

#[test]
fn large_shape_json_freed_by_the_cap_does_not_cost_extra_usage_rows() {
    let db = HistoryDb::open_in_memory().unwrap();
    fill_usage(&db, 1_500, NOW - 90_000);
    let rows: Vec<_> = (0..1_200)
        .map(|s| {
            let mut o = obs(
                key(1, ShapeSource::App, &format!("shape-{s}")),
                NOW - 3_600,
                1,
                0,
                (1, 1),
                VerdictKind::IndexUsed,
            );
            o.shape_json = "x".repeat(2_500);
            o
        })
        .collect();
    db.record_shapes(&rows).unwrap();
    let before = count(&db, "SELECT COUNT(*) FROM samples");
    // About one batch of shapes (500 x 2.5 KB) has to go.
    let cap = db.used_bytes().unwrap() - 1_000_000;
    let db = db.with_max_bytes(cap);
    thin(&db);
    assert!(db.used_bytes().unwrap() <= cap);
    let lost = before - count(&db, "SELECT COUNT(*) FROM samples");
    assert!(lost <= 500, "usage lost {lost} rows, more than one batch");
}

fn lost_rows(db: HistoryDb, delta: u64) -> (i64, i64) {
    let rows = |db: &HistoryDb| {
        (
            count(db, "SELECT COUNT(*) FROM samples"),
            count(db, "SELECT COUNT(*) FROM shape_bucket"),
        )
    };
    let before = rows(&db);
    let cap = db.used_bytes().unwrap() - delta;
    let db = db.with_max_bytes(cap);
    thin(&db);
    let after = rows(&db);
    (before.0 - after.0, before.1 - after.1)
}

#[test]
fn the_older_set_loses_rows_first() {
    // Usage is 20 hours older than the shapes: a small excess is taken from usage alone.
    let db = HistoryDb::open_in_memory().unwrap();
    fill_usage(&db, 1_200, NOW - 46 * HOUR_SECS);
    fill_shapes(&db, 8, 140, NOW - 23 * HOUR_SECS);
    let (usage, shapes) = lost_rows(db, 8_000);
    assert!(usage > 0 && shapes == 0, "{usage} {shapes}");
    // And the other way round.
    let db = HistoryDb::open_in_memory().unwrap();
    fill_usage(&db, 1_200, NOW - 23 * HOUR_SECS);
    fill_shapes(&db, 8, 140, NOW - 46 * HOUR_SECS);
    let (usage, shapes) = lost_rows(db, 8_000);
    assert!(shapes > 0 && usage == 0, "{usage} {shapes}");
}

#[test]
fn equally_old_sets_lose_comparable_shares() {
    let db = HistoryDb::open_in_memory().unwrap();
    fill_usage(&db, 1_500, NOW - 25 * HOUR_SECS);
    fill_shapes(&db, 8, 150, NOW - 25 * HOUR_SECS);
    let used = db.used_bytes().unwrap();
    let (usage, shapes) = lost_rows(db, used / 3);
    assert!(usage > 0 && shapes > 0, "{usage} {shapes}");
    assert!(
        usage <= 3 * shapes && shapes <= 3 * usage,
        "{usage} {shapes}"
    );
}
