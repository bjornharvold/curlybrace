//! A sample stamped ahead of the wall clock never blocks the real ones.

use super::db::*;
use super::retention::ChartRange;
use super::series::{DEFAULT_MAX_CONTINUOUS_SECS, MAX_FUTURE_SKEW_SECS, Sample};
use uuid::Uuid;

const DAY: i64 = 86_400;
const NOW: i64 = 1_800_000_000 / DAY * DAY;
const MAX: i64 = DEFAULT_MAX_CONTINUOUS_SECS;

fn key() -> SeriesKey {
    SeriesKey {
        connection: Uuid::from_u128(1),
        host: "node-a:27017".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "by_sku".into(),
        definition_hash: "h1".into(),
    }
}

fn s(at_secs: i64, ops: i64) -> Sample {
    Sample {
        at_secs,
        ops,
        since_ms: 1_000,
    }
}

fn uses(db: &HistoryDb, now: i64) -> i64 {
    let buckets = db.usage(&key(), ChartRange::Day, now, MAX).unwrap();
    buckets.iter().map(|b| b.uses).sum()
}

#[test]
fn a_far_future_sample_stored_first_does_not_block_the_real_ones() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    db.record(&k, s(NOW + 10 * DAY, 1)).unwrap();
    db.record(&k, s(NOW - 180, 10)).unwrap();
    db.record(&k, s(NOW - 120, 12)).unwrap();
    db.record(&k, s(NOW - 60, 16)).unwrap();

    assert_eq!(uses(&db, NOW), 6);
    let view = db.series_view(&k, ChartRange::Day, NOW, MAX).unwrap();
    assert_eq!(view.facts.last_used_at_secs, Some(NOW - 60));
    assert_eq!(view.facts.tracked_since_secs, Some(NOW - 180));
    assert_eq!(view.latest, Some(s(NOW - 60, 16)));
    let facts = db.facts(&k, NOW, MAX).unwrap();
    assert_eq!(facts, view.facts);
}

#[test]
fn a_sample_within_the_skew_is_still_read() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record(&key(), s(NOW - 60, 10)).unwrap();
    db.record(&key(), s(NOW + MAX_FUTURE_SKEW_SECS, 14))
        .unwrap();
    assert_eq!(uses(&db, NOW), 4);
}

#[test]
fn a_clock_stepped_back_between_passes_reads_the_new_samples_not_the_old_ones() {
    let db = HistoryDb::open_in_memory().unwrap();
    // Stamped by the old clock, an hour ahead of the corrected one.
    db.record(&key(), s(NOW, 100)).unwrap();
    db.record(&key(), s(NOW + 60, 103)).unwrap();
    let corrected = NOW - 3_600;
    db.record(&key(), s(corrected + 60, 5)).unwrap();
    db.record(&key(), s(corrected + 120, 8)).unwrap();

    assert_eq!(uses(&db, corrected + 120), 3);
}

fn stored_stamps(db: &HistoryDb) -> Vec<i64> {
    let conn = db.conn.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT at_secs FROM samples ORDER BY at_secs")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn series_count(db: &HistoryDb) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row("SELECT COUNT(*) FROM series", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn thinning_keeps_a_sample_ahead_of_the_clock() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    db.record(&k, s(NOW + 10 * DAY, 1)).unwrap();
    db.record(&k, s(NOW - 60, 10)).unwrap();

    db.thin(NOW, MAX).unwrap();

    assert_eq!(stored_stamps(&db), vec![NOW - 60, NOW + 10 * DAY]);
}

#[test]
fn thinning_with_a_clock_one_year_behind_deletes_nothing() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    for (i, ops) in [10, 12, 16].into_iter().enumerate() {
        db.record(&k, s(NOW - 180 + 60 * i as i64, ops)).unwrap();
    }
    let behind = NOW - 365 * DAY;

    db.thin(behind, MAX).unwrap();

    assert_eq!(stored_stamps(&db).len(), 3);
    assert_eq!(series_count(&db), 1, "the series keeps its history");
}

#[test]
fn normal_retention_still_deletes_what_it_did() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    // Older than the two-year daily retention.
    db.record(&k, s(NOW - 3 * 365 * DAY, 1)).unwrap();
    db.record(&k, s(NOW - 60, 10)).unwrap();

    db.thin(NOW, MAX).unwrap();

    assert!(!stored_stamps(&db).contains(&(NOW - 3 * 365 * DAY)));
    assert!(stored_stamps(&db).contains(&(NOW - 60)));
}

#[test]
fn a_series_whose_rows_are_all_ahead_of_the_clock_is_kept() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record(&key(), s(NOW + 10 * DAY, 1)).unwrap();

    db.thin(NOW, MAX).unwrap();

    assert_eq!(series_count(&db), 1);
    assert_eq!(stored_stamps(&db), vec![NOW + 10 * DAY]);
}
