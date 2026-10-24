use super::db::*;
use super::retention::*;
use super::series::{Bucket, DEFAULT_MAX_CONTINUOUS_SECS, Sample, bucketize, derive};
use uuid::Uuid;

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;
const SINCE: i64 = 1_000;
/// A fixed "now", aligned to a day so bucket arithmetic in the tests is easy to read.
const NOW: i64 = 1_800_000_000 / DAY * DAY;

fn key_for(connection: Uuid, host: &str, hash: &str) -> SeriesKey {
    SeriesKey {
        connection,
        host: host.into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "by_sku".into(),
        definition_hash: hash.into(),
    }
}

fn key() -> SeriesKey {
    key_for(Uuid::from_u128(1), "node-a:27017", "h1")
}

fn s(at_secs: i64, ops: i64) -> Sample {
    Sample {
        at_secs,
        ops,
        since_ms: SINCE,
    }
}

/// Records one sample per minute from `from` (inclusive) to `to` (exclusive), the counter
/// growing by 2 each minute from `ops0`. Returns the counter after the last sample.
fn record_minutely(db: &HistoryDb, key: &SeriesKey, from: i64, to: i64, ops0: i64) -> i64 {
    let mut ops = ops0;
    let mut at = from;
    while at < to {
        db.record(key, s(at, ops)).unwrap();
        ops += 2;
        at += 60;
    }
    ops
}

fn totals(buckets: &[Bucket]) -> (i64, u32, i64) {
    buckets.iter().fold((0, 0, 0), |(u, r, g), b| {
        (u + b.uses, r + b.restarts, g + b.gap_secs)
    })
}

fn count(db: &HistoryDb, sql: &str) -> i64 {
    db.conn
        .lock()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

#[test]
fn records_and_reads_back_a_series() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    assert_eq!(db.latest(&k).unwrap(), None);
    db.record(&k, s(NOW - 120, 10)).unwrap();
    db.record(&k, s(NOW - 60, 14)).unwrap();
    assert_eq!(db.latest(&k).unwrap(), Some(s(NOW - 60, 14)));
    assert_eq!(
        db.series_for(k.connection, "shop", "orders").unwrap(),
        vec![k.clone()]
    );
    assert!(
        db.series_for(k.connection, "shop", "other")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn usage_returns_buckets_of_the_range_width() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    let ops = record_minutely(&db, &k, NOW - 3 * HOUR, NOW, 100);
    for range in [ChartRange::Day, ChartRange::Week, ChartRange::Month] {
        let buckets = db
            .usage(&k, range, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap();
        assert!(!buckets.is_empty());
        assert!(
            buckets
                .iter()
                .all(|b| b.width_secs == range.bucket_width_secs())
        );
        // The first sample has no delta, so the total is the counter's growth.
        assert_eq!(totals(&buckets).0, ops - 2 - 100);
    }
    assert_eq!(ChartRange::Day.secs(), DAY);
    assert_eq!(ChartRange::Week.secs(), 7 * DAY);
    assert_eq!(ChartRange::Month.secs(), 30 * DAY);
}

#[test]
fn usage_only_covers_the_requested_window() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    record_minutely(&db, &k, NOW - 10 * HOUR, NOW, 0);
    let day = db
        .usage(&k, ChartRange::Day, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert!(day.iter().all(|b| b.start_secs + b.width_secs > NOW - DAY));
    let narrow = db
        .usage(
            &k,
            ChartRange::Day,
            NOW - 8 * HOUR,
            DEFAULT_MAX_CONTINUOUS_SECS,
        )
        .unwrap();
    assert!(narrow.iter().all(|b| b.start_secs <= NOW - 8 * HOUR));
}

#[test]
fn a_recreated_index_with_a_different_definition_is_a_separate_series() {
    let db = HistoryDb::open_in_memory().unwrap();
    let old = key_for(Uuid::from_u128(1), "node-a:27017", "h1");
    let new = key_for(Uuid::from_u128(1), "node-a:27017", "h2");
    record_minutely(&db, &old, NOW - 2 * HOUR, NOW - HOUR, 500);
    // The recreated index starts again from 0.
    let ops = record_minutely(&db, &new, NOW - HOUR, NOW, 0);
    assert_eq!(
        totals(
            &db.usage(&old, ChartRange::Day, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
                .unwrap()
        )
        .0,
        118
    );
    assert_eq!(
        totals(
            &db.usage(&new, ChartRange::Day, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
                .unwrap()
        )
        .0,
        ops - 2
    );
    assert_eq!(
        db.usage(&new, ChartRange::Day, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap()
            .iter()
            .map(|b| b.restarts)
            .sum::<u32>(),
        0
    );
    let mut listed = db.series_for(old.connection, "shop", "orders").unwrap();
    listed.sort_by(|a, b| a.definition_hash.cmp(&b.definition_hash));
    assert_eq!(listed, vec![old, new]);
}

#[test]
fn a_different_host_is_a_separate_series() {
    let db = HistoryDb::open_in_memory().unwrap();
    let a = key_for(Uuid::from_u128(1), "node-a:27017", "h1");
    let b = key_for(Uuid::from_u128(1), "node-b:27017", "h1");
    record_minutely(&db, &a, NOW - HOUR, NOW, 0);
    db.record(&b, s(NOW - 60, 7)).unwrap();
    assert_eq!(db.latest(&b).unwrap(), Some(s(NOW - 60, 7)));
    assert_eq!(
        totals(
            &db.usage(&b, ChartRange::Day, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
                .unwrap()
        )
        .0,
        0
    );
    assert_eq!(
        db.series_for(a.connection, "shop", "orders").unwrap().len(),
        2
    );
}

#[test]
fn insertion_order_not_timestamp_order_drives_the_series() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    db.record(&k, s(NOW - 180, 10)).unwrap();
    db.record(&k, s(NOW - 120, 12)).unwrap();
    // The clock jumped back: not after the previous sample, so `derive` ignores it.
    db.record(&k, s(NOW - 500, 99)).unwrap();
    db.record(&k, s(NOW - 60, 15)).unwrap();
    // `latest` is the last one inserted, whatever its timestamp.
    db.record(&k, s(NOW - 900, 3)).unwrap();
    assert_eq!(db.latest(&k).unwrap(), Some(s(NOW - 900, 3)));
    // 10 -> 12 -> 15; the skewed samples contribute neither uses nor restarts.
    let (uses, restarts, _) = totals(
        &db.usage(&k, ChartRange::Day, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
    );
    assert_eq!((uses, restarts), (5, 0));
}

#[test]
fn thinning_keeps_restarts_and_gap_seconds_in_hourly_buckets() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    let start = NOW - 4 * DAY + 17 * 60;
    // Continuous, then a restart (since changes), then a two-hour hole, then continuous
    // again right across the 48 hour cutoff.
    let mut at = start;
    let mut ops = 0;
    for _ in 0..120 {
        db.record(&k, s(at, ops)).unwrap();
        at += 60;
        ops += 3;
    }
    for _ in 0..30 {
        db.record(
            &k,
            Sample {
                at_secs: at,
                ops: ops - 300,
                since_ms: SINCE + 1,
            },
        )
        .unwrap();
        at += 60;
        ops += 3;
    }
    at += 2 * HOUR + 40;
    while at < NOW - DAY {
        db.record(
            &k,
            Sample {
                at_secs: at,
                ops,
                since_ms: SINCE + 1,
            },
        )
        .unwrap();
        at += 60;
        ops += 3;
    }
    let before = db
        .usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    let raw_before = count(&db, "SELECT COUNT(*) FROM samples");
    assert!(totals(&before).1 >= 1 && totals(&before).2 >= 2 * HOUR);

    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();

    assert!(count(&db, "SELECT COUNT(*) FROM samples") < raw_before);
    assert!(count(&db, "SELECT COUNT(*) FROM rolled WHERE tier = 1") > 0);
    let after = db
        .usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert_eq!(after, before);
    // Thinning again changes nothing: nothing is lost or counted twice.
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert_eq!(
        db.usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
        before
    );
}

#[test]
fn rolling_does_not_lose_or_double_count_uses_at_the_cutoff() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    // Continuous samples straddling the 48 hour cutoff, then a gap whose far end is raw.
    let ops = record_minutely(&db, &k, NOW - 60 * HOUR, NOW - 40 * HOUR, 0);
    let before = db
        .usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert_eq!(totals(&before).0, ops - 2);
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert_eq!(
        db.usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
        before
    );
    // New data arrives after the hourly rows exist, across a gap that starts in rolled time.
    db.record(&k, s(NOW - 10 * HOUR, ops + 500)).unwrap();
    let (uses, _, gap) = totals(
        &db.usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
    );
    // 2 is the last minute's growth the gap point carries: the counter went from ops - 2.
    assert_eq!(uses, ops - 2 + 500 + 2);
    assert_eq!(gap, 30 * HOUR + 60);
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    let (uses2, _, gap2) = totals(
        &db.usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
    );
    assert_eq!((uses2, gap2), (uses, gap));
}

#[test]
fn a_gap_straddling_the_cutoff_is_counted_once() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    // The last rolled sample is just before the 48 hour cutoff, the next one two hours later.
    let ops = record_minutely(&db, &k, NOW - 52 * HOUR, NOW - 49 * HOUR, 0);
    record_minutely(&db, &k, NOW - 47 * HOUR, NOW - 46 * HOUR, ops + 40);
    let before = db
        .usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert!(totals(&before).2 > HOUR);
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert_eq!(
        db.usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
        before
    );
    // A later thinning, with the cutoff moved on, still agrees.
    db.thin(NOW + 5 * HOUR, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert_eq!(
        totals(
            &db.usage(
                &k,
                ChartRange::Week,
                NOW + 5 * HOUR,
                DEFAULT_MAX_CONTINUOUS_SECS
            )
            .unwrap()
        ),
        totals(&before)
    );
}

/// What a chart over the whole series, never thinned, would show.
fn expected_window(samples: &[Sample], range: ChartRange, now: i64) -> Vec<Bucket> {
    let start = now - range.secs();
    bucketize(
        &derive(samples, DEFAULT_MAX_CONTINUOUS_SECS),
        range.bucket_width_secs(),
    )
    .into_iter()
    .filter(|b| b.start_secs + b.width_secs > start && b.start_secs <= now)
    .collect()
}

#[test]
fn a_silence_spanning_a_thin_keeps_its_gap_seconds() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    let mut all = Vec::new();
    let mut at = NOW - 104 * HOUR;
    while at <= NOW - 100 * HOUR {
        all.push(s(at, at / 60));
        at += 60;
    }
    for sample in &all {
        db.record(&k, *sample).unwrap();
    }
    db.thin(NOW - 50 * HOUR, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    let last = s(NOW - 60, all.last().unwrap().ops + 700);
    db.record(&k, last).unwrap();
    all.push(last);
    for range in [ChartRange::Week, ChartRange::Month] {
        assert_eq!(
            db.usage(&k, range, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
                .unwrap(),
            expected_window(&all, range, NOW)
        );
    }
    let before = db
        .usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert_eq!(
        db.usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
        before
    );
}

#[test]
fn a_backwards_clock_sample_never_decides_what_thin_deletes() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    let mut all = Vec::new();
    let mut at = NOW - 47 * HOUR;
    while at < NOW - 60 {
        all.push(s(at, at / 60));
        at += 60;
    }
    all.push(s(NOW - 50 * HOUR, 1));
    for sample in &all {
        db.record(&k, *sample).unwrap();
    }
    let before = db
        .usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    let rows = count(&db, "SELECT COUNT(*) FROM samples");
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples"), rows);
    assert_eq!(
        db.usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap(),
        before
    );
    assert_eq!(before, expected_window(&all, ChartRange::Week, NOW));
}

#[test]
fn thinned_buckets_match_derive_and_bucketize_of_the_full_series() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    let mut samples = Vec::new();
    let mut at = NOW - 5 * DAY;
    let mut ops = 0;
    while at < NOW - 30 * 60 {
        // A hole of 20 minutes every so often.
        at += if samples.len() % 400 == 399 {
            1_200
        } else {
            60
        };
        ops += 1 + (samples.len() as i64 % 5);
        samples.push(s(at, ops));
    }
    for sample in &samples {
        db.record(&k, *sample).unwrap();
    }
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    let expected = bucketize(&derive(&samples, DEFAULT_MAX_CONTINUOUS_SECS), HOUR);
    let got = db
        .usage(&k, ChartRange::Week, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert_eq!(totals(&got), totals(&expected));
}

#[test]
fn thinning_drops_daily_rows_older_than_two_years() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    // About three years old, and about one year old.
    record_minutely(&db, &k, NOW - 3 * 365 * DAY, NOW - 3 * 365 * DAY + HOUR, 0);
    record_minutely(&db, &k, NOW - 365 * DAY, NOW - 365 * DAY + HOUR, 0);
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    // Only the year-old data is left, as daily rows; the raw and hourly rows are gone.
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM rolled WHERE tier = 1"), 0);
    assert!(count(&db, "SELECT COUNT(*) FROM rolled WHERE tier = 2") >= 1);
    assert_eq!(
        count(
            &db,
            &format!(
                "SELECT COUNT(*) FROM rolled WHERE start_secs < {}",
                NOW - 2 * 365 * DAY
            )
        ),
        0
    );
    // Once even the daily rows expire, the series is forgotten entirely.
    db.thin(NOW + 2 * 365 * DAY, DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM rolled"), 0);
    assert!(
        db.series_for(k.connection, "shop", "orders")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_size_cap_removes_the_oldest_rows_first() {
    let db = HistoryDb::open_in_memory().unwrap().with_max_bytes(80_000);
    let k = key();
    record_minutely(&db, &k, NOW - 40 * HOUR, NOW, 0);
    let oldest = |db: &HistoryDb| count(db, "SELECT MIN(at_secs) FROM samples");
    let first = oldest(&db);
    let rows = count(&db, "SELECT COUNT(*) FROM samples");
    assert!(db.used_bytes().unwrap() > 80_000);
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert!(db.used_bytes().unwrap() <= 80_000);
    assert!(count(&db, "SELECT COUNT(*) FROM samples") < rows);
    assert!(oldest(&db) > first);
    // The newest sample survives.
    assert_eq!(db.latest(&k).unwrap().map(|x| x.at_secs), Some(NOW - 60));
}

#[test]
fn the_size_cap_drops_daily_rows_before_hourly_before_raw() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    // A long silence fills many hourly and daily rows; a recent hour stays raw.
    record_minutely(&db, &k, NOW - 600 * DAY, NOW - 600 * DAY + HOUR, 0);
    record_minutely(&db, &k, NOW - 3 * HOUR, NOW, 0);
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    let tier = |db: &HistoryDb, t: i64| {
        count(db, &format!("SELECT COUNT(*) FROM rolled WHERE tier = {t}"))
    };
    let (daily, hourly) = (tier(&db, 2), tier(&db, 1));
    let raw = count(&db, "SELECT COUNT(*) FROM samples");
    assert!(daily > 300 && hourly > 300 && raw > 0);
    // A cap one page below the current size is met by the daily rows alone.
    let capped = db.with_max_bytes_from_used(4_096);
    capped.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert!(tier(&capped, 2) < daily);
    assert_eq!(tier(&capped, 1), hourly);
    assert_eq!(count(&capped, "SELECT COUNT(*) FROM samples"), raw);
    // A cap nothing can meet empties every tier, oldest tier first, without erroring.
    let empty = capped.with_max_bytes(1);
    empty.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert_eq!(tier(&empty, 2) + tier(&empty, 1), 0);
    assert_eq!(count(&empty, "SELECT COUNT(*) FROM samples"), 0);
}

#[test]
fn purge_connection_removes_only_that_connections_rows() {
    let db = HistoryDb::open_in_memory().unwrap();
    let a = key_for(Uuid::from_u128(1), "h", "x");
    let b = key_for(Uuid::from_u128(2), "h", "x");
    record_minutely(&db, &a, NOW - HOUR, NOW, 0);
    record_minutely(&db, &b, NOW - HOUR, NOW, 0);
    record_minutely(&db, &a, NOW - 100 * DAY, NOW - 100 * DAY + HOUR, 0);
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    db.purge_connection(a.connection).unwrap();
    assert_eq!(db.latest(&a).unwrap(), None);
    assert!(
        db.usage(&a, ChartRange::Month, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
            .unwrap()
            .is_empty()
    );
    assert!(
        db.series_for(a.connection, "shop", "orders")
            .unwrap()
            .is_empty()
    );
    assert!(db.latest(&b).unwrap().is_some());
    assert_eq!(
        db.series_for(b.connection, "shop", "orders").unwrap().len(),
        1
    );
    assert_eq!(
        count(&db, "SELECT COUNT(DISTINCT series_id) FROM samples"),
        1
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM series"), 1);
}

#[cfg(unix)]
#[test]
fn the_file_is_created_with_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert_eq!(recovery, None);
    db.record(&key(), s(NOW, 1)).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_file_database_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let (db, _) = HistoryDb::open(&path).unwrap();
        db.record(&key(), s(NOW, 1)).unwrap();
    }
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert_eq!(recovery, None);
    assert_eq!(db.latest(&key()).unwrap(), Some(s(NOW, 1)));
}

#[test]
fn a_corrupt_file_is_set_aside_and_a_fresh_one_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let garbage = vec![0xAB_u8; 4_096];
    std::fs::write(&path, &garbage).unwrap();
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    let aside = dir.path().join("history.sqlite.corrupt");
    assert_eq!(recovery, Some(Recovery::SetAside(aside.clone())));
    assert_eq!(std::fs::read(&aside).unwrap(), garbage);
    db.record(&key(), s(NOW, 1)).unwrap();
    assert_eq!(db.latest(&key()).unwrap(), Some(s(NOW, 1)));
    // A second corruption never overwrites the first set-aside file.
    drop(db);
    std::fs::write(&path, &garbage).unwrap();
    let (_db, recovery) = HistoryDb::open(&path).unwrap();
    let Some(Recovery::SetAside(second)) = recovery else {
        panic!("expected a set-aside");
    };
    assert_ne!(second, aside);
    assert!(aside.exists() && second.exists());
}

#[test]
fn a_newer_schema_version_is_refused_not_set_aside() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    drop(HistoryDb::open(&path).unwrap());
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
    }
    assert!(HistoryDb::open(&path).is_err());
    assert!(path.exists());
    assert!(!dir.path().join("history.sqlite.corrupt").exists());
}

#[test]
fn the_db_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<HistoryDb>();
}

#[test]
fn record_many_stores_every_sample_like_separate_records() {
    let db = HistoryDb::open_in_memory().unwrap();
    let other = key_for(Uuid::from_u128(1), "node-b:27017", "h1");
    db.record_many(&[
        (key(), s(100, 1)),
        (other.clone(), s(100, 5)),
        (key(), s(160, 3)),
    ])
    .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM series"), 2);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples"), 3);
    assert_eq!(db.latest(&key()).unwrap(), Some(s(160, 3)));
    assert_eq!(db.latest(&other).unwrap(), Some(s(100, 5)));
}

#[test]
fn record_many_is_all_or_nothing() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse BEFORE INSERT ON samples WHEN NEW.ops < 0
             BEGIN SELECT RAISE(ABORT, 'refused'); END",
        )
        .unwrap();
    let result = db.record_many(&[(key(), s(100, 1)), (key(), s(160, -1))]);
    assert!(result.is_err());
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM series"), 0);
}

#[test]
fn facts_are_the_real_time_of_the_last_use_and_of_the_first_sample() {
    let db = HistoryDb::open_in_memory().unwrap();
    let key = key();
    let first = NOW - 2 * HOUR;
    let used = NOW - 59 * 60;
    // One sample a minute; the counter moves once, at `used`, and then stands still.
    let mut at = first;
    while at < NOW {
        db.record(&key, s(at, if at >= used { 3 } else { 0 }))
            .unwrap();
        at += 60;
    }

    let facts = db.facts(&key, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();

    assert_eq!(facts.last_used_at_secs, Some(used));
    assert_eq!(facts.tracked_since_secs, Some(first));
}

#[test]
fn facts_of_an_unused_or_unknown_series_have_no_last_use() {
    let db = HistoryDb::open_in_memory().unwrap();
    assert_eq!(
        db.facts(&key(), DEFAULT_MAX_CONTINUOUS_SECS).unwrap(),
        SeriesFacts::default()
    );
    for minute in 0..5 {
        db.record(&key(), s(NOW - 600 + minute * 60, 7)).unwrap();
    }
    let facts = db.facts(&key(), DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
    assert_eq!(facts.last_used_at_secs, None);
    assert_eq!(facts.tracked_since_secs, Some(NOW - 600));
}

#[test]
fn facts_survive_thinning_without_depending_on_a_range() {
    let db = HistoryDb::open_in_memory().unwrap();
    let key = key();
    let start = NOW - 10 * DAY;
    // Used every minute until 5 days ago, idle since (the counter stops).
    let used_until = NOW - 5 * DAY;
    let mut ops = 0;
    let mut at = start;
    while at < NOW {
        if at < used_until {
            ops += 2;
        }
        db.record(&key, s(at, ops)).unwrap();
        at += 10 * 60;
    }
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();

    let facts = db.facts(&key, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();

    let last = facts.last_used_at_secs.expect("rolled uses remain");
    assert!(
        (used_until - HOUR..=used_until + DAY).contains(&last),
        "within the rolled row of the last use: {last}"
    );
    let since = facts.tracked_since_secs.expect("rolled rows remain");
    assert!(since <= start + DAY && since >= start - DAY, "{since}");
}

#[test]
fn series_view_is_the_same_as_facts_usage_and_latest_read_separately() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key();
    let max = DEFAULT_MAX_CONTINUOUS_SECS;
    // An unknown series has nothing in any of the three.
    let none = db.series_view(&k, ChartRange::Day, NOW, max).unwrap();
    assert_eq!(none.facts, SeriesFacts::default());
    assert!(none.buckets.is_empty() && none.latest.is_none());

    // Four days of samples with a restart, thinned so rolled rows exist beside raw ones.
    let mut at = NOW - 4 * DAY;
    let mut ops = 0;
    while at < NOW {
        let since_ms = if at < NOW - 2 * DAY { SINCE } else { SINCE + 1 };
        db.record(
            &k,
            Sample {
                at_secs: at,
                ops,
                since_ms,
            },
        )
        .unwrap();
        at += 60;
        ops += 2;
    }
    db.thin(NOW, max).unwrap();

    for range in [ChartRange::Day, ChartRange::Week, ChartRange::Month] {
        let view = db.series_view(&k, range, NOW, max).unwrap();
        assert_eq!(view.facts, db.facts(&k, max).unwrap(), "{range:?}");
        assert_eq!(
            view.buckets,
            db.usage(&k, range, NOW, max).unwrap(),
            "{range:?}"
        );
        assert_eq!(view.latest, db.latest(&k).unwrap(), "{range:?}");
    }
}
