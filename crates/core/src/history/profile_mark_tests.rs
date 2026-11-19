//! The persisted profile watermark: it moves with the shapes it stands for, atomically, and
//! survives a reopened file.

use super::db::HistoryDb;
use super::db_schema::{migrate_to_v1, migrate_to_v2, prepare};
use super::profile_mark::ProfileMark;
use super::shapes::ShapeSource;
use super::shapes_tests::{NOW, conn_id, count, key, obs};
use crate::explain::VerdictKind;
use rusqlite::Connection;

fn mark(ts_millis: i64) -> ProfileMark {
    ProfileMark {
        connection: conn_id(1),
        host: "node-a:27017".into(),
        db: "shop".into(),
        ts_millis,
    }
}

fn one_row() -> Vec<super::shapes::ShapeObservation> {
    vec![obs(
        key(1, ShapeSource::App, "a"),
        NOW,
        1,
        0,
        (1, 1),
        VerdictKind::IndexUsed,
    )]
}

#[test]
fn record_page_records_the_rows_and_moves_the_mark() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_page(&one_row(), Some(&mark(5_000))).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_bucket"), 1);
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), Some(5_000));
}

#[test]
fn a_page_without_rows_still_moves_the_mark() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_page(&[], Some(&mark(7_000))).unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), Some(7_000));
}

#[test]
fn the_mark_only_moves_forward() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_page(&[], Some(&mark(9_000))).unwrap();
    db.record_page(&[], Some(&mark(4_000))).unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), Some(9_000));
}

#[test]
fn no_mark_is_none_and_the_latest_host_wins_by_maximum() {
    let db = HistoryDb::open_in_memory().unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), None);
    db.record_page(&[], Some(&mark(3_000))).unwrap();
    let other = ProfileMark {
        host: "node-b:27017".into(),
        ..mark(8_000)
    };
    db.record_page(&[], Some(&other)).unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), Some(8_000));
    assert_eq!(db.profile_mark(conn_id(1), "other").unwrap(), None);
    assert_eq!(db.profile_mark(conn_id(2), "shop").unwrap(), None);
}

#[test]
fn mark_survives_a_reopened_history_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let (db, _) = HistoryDb::open(&path).unwrap();
        db.record_page(&one_row(), Some(&mark(12_000))).unwrap();
    }
    let (db, _) = HistoryDb::open(&path).unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), Some(12_000));
}

#[test]
fn record_page_is_atomic_when_the_mark_write_fails() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.exec_for_tests("ALTER TABLE profile_mark RENAME TO profile_mark_away");
    assert!(db.record_page(&one_row(), Some(&mark(5_000))).is_err());
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_bucket"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 0);
    db.exec_for_tests("ALTER TABLE profile_mark_away RENAME TO profile_mark");
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), None);
}

#[test]
fn a_purged_connection_records_neither_rows_nor_mark() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.mark_purged(conn_id(1));
    db.record_page(&one_row(), Some(&mark(5_000))).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_bucket"), 0);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM profile_mark"), 0);
}

#[test]
fn purge_removes_marks() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_page(&[], Some(&mark(5_000))).unwrap();
    let kept = ProfileMark {
        connection: conn_id(2),
        ..mark(6_000)
    };
    db.record_page(&[], Some(&kept)).unwrap();
    db.purge_connection(conn_id(1)).unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), None);
    assert_eq!(db.profile_mark(conn_id(2), "shop").unwrap(), Some(6_000));
}

#[test]
fn thinning_drops_marks_older_than_the_raw_retention() {
    let db = HistoryDb::open_in_memory().unwrap();
    let old = (NOW - super::retention::RAW_RETENTION_SECS - 60) * 1_000;
    let fresh = (NOW - 60) * 1_000;
    db.record_page(&[], Some(&mark(old))).unwrap();
    let fresh_mark = ProfileMark {
        db: "fresh".into(),
        ..mark(fresh)
    };
    db.record_page(&[], Some(&fresh_mark)).unwrap();
    db.thin(NOW, super::series::DEFAULT_MAX_CONTINUOUS_SECS)
        .unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), None);
    assert_eq!(db.profile_mark(conn_id(1), "fresh").unwrap(), Some(fresh));
}

#[test]
fn v2_file_without_the_mark_table_gets_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        // A v2 file as an earlier build of this branch wrote it: no `profile_mark`.
        let mut conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate_to_v1(&mut conn).unwrap();
        migrate_to_v2(&mut conn).unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS profile_mark")
            .unwrap();
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'profile_mark'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
    }
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert!(recovery.is_none());
    db.record_page(&[], Some(&mark(1_000))).unwrap();
    assert_eq!(db.profile_mark(conn_id(1), "shop").unwrap(), Some(1_000));
    // Preparing again is a no-op.
    drop(db);
    let mut conn = Connection::open(&path).unwrap();
    prepare(&mut conn).unwrap();
}
