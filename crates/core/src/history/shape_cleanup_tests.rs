//! The one-time cleanup of recorded shapes that no longer validate (an operator the allowlist does
//! not know): once per file, in one transaction, no schema version bump, and never fatal for a
//! file it cannot write.

use rusqlite::Connection;

use super::db::HistoryDb;
use super::db_schema::{SCHEMA_VERSION, migrate_to_v1, prepare};
use super::shape_cleanup::run_once;

const GOOD: &str = r#"{"filter":{"a":{"$gt":"?"}}}"#;
const GOOD_REGEX: &str = r#"{"filter":{"a":{"$regex":"?"},"b":{"$in":["?","$regex"]}}}"#;
const OLD_UNKNOWN: &str = r#"{"filter":{"a":{"$secretkey":"?"}}}"#;

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn insert(conn: &Connection, id: i64, json: &str) {
    conn.execute(
        "INSERT INTO shape (id, connection, host, db, collection, hash, source, shape_json)
         VALUES (?1, 'c', 'h', 'shop', 'orders', ?2, 'app', ?3)",
        rusqlite::params![id, format!("hash{id}"), json],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO shape_bucket
            (shape_id, tier, start_secs, executions, misses, total_millis, max_millis, worst, gap_secs)
         VALUES (?1, 0, 600, 3, 1, 30, 20, 'collscan', 0)",
        [id],
    )
    .unwrap();
}

/// A prepared file with the cleanup marker removed, as a file from before the cleanup is.
fn file_before_cleanup(dir: &tempfile::TempDir) -> Connection {
    let mut conn = Connection::open(dir.path().join("history.sqlite")).unwrap();
    prepare(&mut conn).unwrap();
    conn.execute_batch("DROP TABLE IF EXISTS shape_cleanup")
        .unwrap();
    conn
}

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

#[test]
fn cleanup_deletes_invalid_shapes_and_their_buckets() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    insert(&conn, 1, OLD_UNKNOWN);
    insert(&conn, 2, "not json at all");
    run_once(&mut conn).unwrap();
    assert_eq!(
        (count(&conn, "shape"), count(&conn, "shape_bucket")),
        (0, 0)
    );
}

#[test]
fn cleanup_keeps_valid_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    insert(&conn, 1, GOOD);
    insert(&conn, 2, GOOD_REGEX);
    insert(&conn, 3, OLD_UNKNOWN);
    run_once(&mut conn).unwrap();
    let kept: Vec<i64> = conn
        .prepare("SELECT id FROM shape ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(kept, vec![1, 2]);
    assert_eq!(count(&conn, "shape_bucket"), 2);
}

#[test]
fn cleanup_keeps_a_within_shape() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    insert(&conn, 1, r#"{"filter":{"loc":{"$within":"?"}}}"#);
    insert(&conn, 2, r#"{"filter":{"loc":{"$box":"?"}}}"#);
    run_once(&mut conn).unwrap();
    let kept: i64 = conn
        .query_row("SELECT id FROM shape", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, 1);
}

#[test]
fn cleanup_keeps_rows_with_root_only_keys_in_an_elem_match() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    for (id, key) in ["$comment", "$expr", "$jsonSchema", "$text", "$where"]
        .iter()
        .enumerate()
    {
        let json =
            format!(r#"{{"filter":{{"a":{{"$elemMatch":{{"{key}":"?","qty":{{"$gt":"?"}}}}}}}}}}"#);
        insert(&conn, id as i64 + 1, &json);
    }
    run_once(&mut conn).unwrap();
    assert_eq!(count(&conn, "shape"), 5);
}

#[test]
fn cleanup_is_bounded_per_batch_and_handles_many_rows() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    for id in 1..=2_500 {
        insert(&conn, id, if id % 2 == 0 { GOOD } else { OLD_UNKNOWN });
    }
    run_once(&mut conn).unwrap();
    assert_eq!(count(&conn, "shape"), 1_250);
}

#[test]
fn cleanup_runs_once_per_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    run_once(&mut conn).unwrap();
    // A row added after the first run is not scanned: the marker says the rule is done.
    insert(&conn, 1, OLD_UNKNOWN);
    run_once(&mut conn).unwrap();
    assert_eq!(count(&conn, "shape"), 1);
}

/// A file with one invalid and one valid shape, closed again.
fn file_with_old_rows(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("history.sqlite");
    {
        let conn = file_before_cleanup(dir);
        insert(&conn, 1, OLD_UNKNOWN);
        insert(&conn, 2, GOOD);
    }
    path
}

#[test]
fn opening_the_file_does_not_run_the_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with_old_rows(&dir);
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert!(recovery.is_none());
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(count(&conn, "shape"), 2, "startup never waits for a scan");
    let marker: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'shape_cleanup'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker, 0);
}

#[test]
fn an_open_history_is_cleaned_once_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with_old_rows(&dir);
    let (db, _) = HistoryDb::open(&path).unwrap();
    // A read before the cleanup sees the rows as an older build wrote them; nothing re-validates
    // them on read.
    let ns_rows = db
        .shapes_for(
            uuid::Uuid::nil(),
            "shop",
            "orders",
            crate::history::ChartRange::Day,
            0,
        )
        .unwrap();
    assert!(ns_rows.is_empty());
    db.clean_shapes_once().unwrap();
    db.clean_shapes_once().unwrap();
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(count(&conn, "shape"), 1);
    assert_eq!(count(&conn, "shape_cleanup"), 1);
}

#[test]
fn a_locked_file_skips_the_cleanup_quickly_and_keeps_the_normal_wait() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with_old_rows(&dir);
    let (db, _) = HistoryDb::open(&path).unwrap();
    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = std::time::Instant::now();
    db.clean_shapes_once().unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the cleanup held the history for {:?}",
        started.elapsed()
    );
    holder.execute_batch("ROLLBACK").unwrap();
    // Skipped, so the next request does it; and the connection's own wait is back to normal.
    db.clean_shapes_once().unwrap();
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(count(&conn, "shape"), 1);
}

/// Timing on a large synthetic history: `cargo test -p curlybrace-core --lib timing -- --ignored
/// --nocapture`. Not a pass/fail check of speed, only of the result.
#[test]
#[ignore = "timing on 100 000 rows; run by hand"]
fn timing_of_the_cleanup_on_a_large_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    {
        let tx = conn.transaction().unwrap();
        for id in 1..=100_000_i64 {
            let json = if id % 10 == 0 { OLD_UNKNOWN } else { GOOD };
            tx.execute(
                "INSERT INTO shape (id, connection, host, db, collection, hash, source, shape_json)
                 VALUES (?1, 'c', 'h', 'shop', 'orders', ?2, 'app', ?3)",
                rusqlite::params![id, format!("hash{id}"), json],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO shape_bucket
                    (shape_id, tier, start_secs, executions, misses, total_millis, max_millis,
                     worst, gap_secs)
                 VALUES (?1, 0, 600, 3, 1, 30, 20, 'collscan', 0)",
                [id],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let started = std::time::Instant::now();
    run_once(&mut conn).unwrap();
    println!(
        "cleanup of 100000 shapes (10000 invalid): {:?}",
        started.elapsed()
    );
    assert_eq!(count(&conn, "shape"), 90_000);
}

#[test]
fn cleanup_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    insert(&conn, 1, GOOD);
    run_once(&mut conn).unwrap();
    conn.execute_batch("DELETE FROM shape_cleanup").unwrap();
    run_once(&mut conn).unwrap();
    assert_eq!(count(&conn, "shape"), 1);
    assert_eq!(count(&conn, "shape_cleanup"), 1);
}

#[test]
fn cleanup_leaves_user_version_at_3() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = file_before_cleanup(&dir);
    run_once(&mut conn).unwrap();
    assert_eq!(user_version(&conn), 3);
    assert_eq!(SCHEMA_VERSION, 3);
}

#[test]
fn cleanup_on_a_v1_file_migrates_then_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let mut conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate_to_v1(&mut conn).unwrap();
    }
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert!(recovery.is_none());
    db.clean_shapes_once().unwrap();
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 3);
    assert_eq!(count(&conn, "shape_cleanup"), 1);
}

#[test]
fn a_read_only_file_still_opens_and_is_cleaned_later() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let conn = file_before_cleanup(&dir);
        insert(&conn, 1, OLD_UNKNOWN);
        conn.execute_batch("PRAGMA journal_mode = DELETE").unwrap();
        drop(conn);
    }
    let mut read_only = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    // The marker cannot be written: the cleanup is skipped, not fatal.
    run_once(&mut read_only).unwrap();
    assert_eq!(count(&read_only, "shape"), 1);
    drop(read_only);
    // The next writable open does it.
    let mut conn = Connection::open(&path).unwrap();
    run_once(&mut conn).unwrap();
    assert_eq!(count(&conn, "shape"), 0);
}

#[test]
fn a_wal_file_holds_no_deleted_shape_after_the_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let mut conn = file_before_cleanup(&dir);
    // A file an earlier run left in WAL mode (the mode is persistent): the deleted pages would
    // sit in the log until a checkpoint.
    let _: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
        .unwrap();
    conn.execute_batch("PRAGMA wal_autocheckpoint = 0").unwrap();
    insert(&conn, 1, OLD_UNKNOWN);
    run_once(&mut conn).unwrap();
    for suffix in ["", "-wal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let bytes = std::fs::read(std::path::PathBuf::from(name)).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("secretkey"),
            "the deleted shape is still in the file{suffix}"
        );
    }
}

#[test]
fn the_cleanup_never_leaves_a_value_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let mut conn = file_before_cleanup(&dir);
        insert(&conn, 1, OLD_UNKNOWN);
        run_once(&mut conn).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("secretkey"),
        "the deleted shape is still in the file"
    );
}
