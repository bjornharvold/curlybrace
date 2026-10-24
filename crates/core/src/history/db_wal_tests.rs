//! The file database runs in WAL mode so a pass of many small commits does not fsync each one,
//! and its sidecar files are as private as the database.

use super::db::*;
use super::series::Sample;
use crate::error::CoreError;
use uuid::Uuid;

fn key() -> SeriesKey {
    SeriesKey {
        connection: Uuid::from_u128(1),
        host: "h".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "by_sku".into(),
        definition_hash: "h1".into(),
    }
}

fn sample() -> Sample {
    Sample {
        at_secs: 1_800_000_000,
        ops: 1,
        since_ms: 1_000,
    }
}

fn pragma_text(db: &HistoryDb, name: &str) -> String {
    db.conn
        .lock()
        .unwrap()
        .pragma_query_value(None, name, |r| r.get::<_, String>(0))
        .unwrap()
}

#[test]
fn a_file_database_uses_wal_with_normal_synchronous() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = HistoryDb::open(&dir.path().join("history.sqlite")).unwrap();
    assert_eq!(pragma_text(&db, "journal_mode"), "wal");
    let synchronous: i64 = db
        .conn
        .lock()
        .unwrap()
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .unwrap();
    assert_eq!(synchronous, 1, "NORMAL");
    // Incremental auto-vacuum still took effect on the fresh file.
    let vacuum: i64 = db
        .conn
        .lock()
        .unwrap()
        .pragma_query_value(None, "auto_vacuum", |r| r.get(0))
        .unwrap();
    assert_eq!(vacuum, 2, "INCREMENTAL");
}

#[test]
fn an_in_memory_database_still_opens_and_records() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record(&key(), sample()).unwrap();
    assert_eq!(db.latest(&key()).unwrap(), Some(sample()));
}

#[cfg(unix)]
#[test]
fn the_database_and_its_wal_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let (db, _) = HistoryDb::open(&path).unwrap();
    db.record(&key(), sample()).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let file = dir.path().join(format!("history.sqlite{suffix}"));
        let mode = std::fs::metadata(&file)
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()))
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{}", file.display());
    }
}

#[test]
fn a_stale_wal_of_a_corrupt_database_does_not_reach_the_fresh_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    std::fs::write(&path, vec![0xAB_u8; 4_096]).unwrap();
    std::fs::write(dir.path().join("history.sqlite-wal"), b"stale wal").unwrap();
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert!(recovery.is_some());
    db.record(&key(), sample()).unwrap();
    assert_eq!(db.latest(&key()).unwrap(), Some(sample()));
    let wal = std::fs::read(dir.path().join("history.sqlite-wal")).unwrap();
    assert!(!wal.windows(9).any(|w| w == b"stale wal"));
}

fn busy() -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY), None)
}

fn locked() -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_LOCKED),
        None,
    )
}

#[test]
fn busy_and_locked_errors_map_to_a_busy_history_error_and_others_do_not() {
    assert!(is_history_busy(&sql_err(busy())));
    assert!(is_history_busy(&sql_err(locked())));
    let full =
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL), None);
    assert!(!is_history_busy(&sql_err(full)));
    assert!(!is_history_busy(&sql_err(
        rusqlite::Error::QueryReturnedNoRows
    )));
    assert!(!is_history_busy(&CoreError::Database("boom".into())));
}

#[test]
fn the_file_database_waits_a_few_seconds_for_a_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = HistoryDb::open(&dir.path().join("history.sqlite")).unwrap();
    let wait: i64 = db
        .conn
        .lock()
        .unwrap()
        .pragma_query_value(None, "busy_timeout", |r| r.get(0))
        .unwrap();
    assert_eq!(wait, BUSY_TIMEOUT.as_millis() as i64);
    assert!(BUSY_TIMEOUT >= std::time::Duration::from_secs(5));
}

#[test]
fn a_write_against_a_held_lock_fails_as_busy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let (db, _) = HistoryDb::open(&path).unwrap();
    db.set_busy_timeout(std::time::Duration::from_millis(20));
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let error = db.record(&key(), sample()).unwrap_err();
    assert!(is_history_busy(&error), "{error:?}");
    other.execute_batch("ROLLBACK").unwrap();
    db.record(&key(), sample()).unwrap();
}
