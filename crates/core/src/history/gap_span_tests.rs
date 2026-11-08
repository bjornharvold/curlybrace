//! `record_gap_span`: parts lie inside one 10-minute bucket, one transaction, purge-safe.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::HistoryDb;

const CONN: Uuid = Uuid::from_u128(3);

fn gaps(db: &HistoryDb) -> Vec<(i64, i64)> {
    let conn = db.lock();
    let mut stmt = conn
        .prepare("SELECT start_secs, gap_secs FROM shape_gap ORDER BY start_secs")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn span(db: &HistoryDb, from: i64, secs: u64) {
    db.record_gap_span(CONN, "h:1", "shop", from, secs).unwrap();
}

#[test]
fn an_unaligned_span_fills_each_bucket_with_what_it_covers() {
    let db = HistoryDb::open_in_memory().unwrap();
    span(&db, 450, 600);
    assert_eq!(gaps(&db), vec![(0, 150), (600, 450)]);
}

#[test]
fn a_span_inside_one_bucket_stays_in_it() {
    let db = HistoryDb::open_in_memory().unwrap();
    span(&db, 1_300, 100);
    assert_eq!(gaps(&db), vec![(1_200, 100)]);
}

#[test]
fn an_aligned_span_gives_whole_buckets_and_a_remainder() {
    let db = HistoryDb::open_in_memory().unwrap();
    span(&db, 1_200, 1_799);
    assert_eq!(gaps(&db), vec![(1_200, 600), (1_800, 600), (2_400, 599)]);
}

#[test]
fn a_span_past_the_retention_keeps_only_its_newest_part() {
    let db = HistoryDb::open_in_memory().unwrap();
    let hundred_hours = 100 * 3_600;
    span(&db, 0, hundred_hours);
    let rows = gaps(&db);
    assert_eq!(rows.len(), 288);
    assert_eq!(rows.iter().map(|r| r.1).sum::<i64>(), 48 * 3_600);
    assert_eq!(rows.last().unwrap().0, hundred_hours as i64 - 600);
}

#[test]
fn a_purged_connection_gets_no_gap_rows() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.purge_connection(CONN).unwrap();
    span(&db, 0, 1_200);
    assert!(gaps(&db).is_empty());
}

#[test]
fn a_write_waiting_for_the_lock_while_a_purge_runs_writes_nothing() {
    let db = Arc::new(HistoryDb::open_in_memory().unwrap());
    let guard = db.lock();
    let writer = {
        let db = Arc::clone(&db);
        std::thread::spawn(move || db.record_gap_span(CONN, "h:1", "shop", 0, 1_200))
    };
    // The writer is parked on the lock (a check made before it would pass); the purge marks the
    // connection while it holds the lock, as `purge_connection` does.
    std::thread::sleep(Duration::from_millis(200));
    db.mark_purged(CONN);
    drop(guard);
    writer.join().unwrap().unwrap();
    assert!(gaps(&db).is_empty());
}
