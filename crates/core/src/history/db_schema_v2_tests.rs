//! The v1 to v2 migration: shape tables appear, nothing of v1 is touched.

use super::db::HistoryDb;
use super::db_schema::{SCHEMA_VERSION, migrate_to_v1, migrate_to_v2, prepare};
use rusqlite::Connection;

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn tables(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn v1_file_migrates_to_v2_without_touching_v1_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        // A real v1 file: written by the previous schema code path, with data in every table.
        let mut conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate_to_v1(&mut conn).unwrap();
        assert_eq!(user_version(&conn), 1);
        conn.execute_batch(
            "INSERT INTO series (id, connection, host, db, collection, index_name, definition_hash)
                 VALUES (1, 'c', 'h', 'shop', 'orders', 'by_sku', 'x');
             INSERT INTO samples (series_id, at_secs, ops, since_ms) VALUES (1, 100, 5, 1000);
             INSERT INTO rolled (series_id, tier, start_secs, uses, restarts, gap_secs)
                 VALUES (1, 1, 3600, 7, 1, 30);",
        )
        .unwrap();
    }
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert!(recovery.is_none());
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 3);
    for table in ["shape", "shape_bucket", "shape_gap"] {
        assert!(tables(&conn).contains(&table.to_owned()), "{table}");
    }
    let one = |sql: &str| -> (i64, i64, i64) {
        conn.query_row(sql, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
    };
    assert_eq!(one("SELECT id, 0, 0 FROM series"), (1, 0, 0));
    assert_eq!(
        one("SELECT at_secs, ops, since_ms FROM samples"),
        (100, 5, 1000)
    );
    assert_eq!(
        one("SELECT uses, restarts, gap_secs FROM rolled"),
        (7, 1, 30)
    );
    // Incremental vacuum, chosen when the v1 file was created, is kept.
    let vacuum: i64 = conn
        .pragma_query_value(None, "auto_vacuum", |r| r.get(0))
        .unwrap();
    assert_eq!(vacuum, 2);
}

#[test]
fn preparing_a_v2_file_again_changes_nothing() {
    let mut conn = Connection::open_in_memory().unwrap();
    prepare(&mut conn).unwrap();
    let before = tables(&conn);
    prepare(&mut conn).unwrap();
    assert_eq!(before, tables(&conn));
    assert_eq!(user_version(&conn), 3);
}

#[test]
fn newer_than_the_current_version_is_refused_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    drop(HistoryDb::open(&path).unwrap());
    {
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();
    }
    assert!(HistoryDb::open(&path).is_err());
    assert!(path.exists());
    assert!(!dir.path().join("history.sqlite.corrupt").exists());
    assert_eq!(user_version(&Connection::open(&path).unwrap()), 4);
}

#[test]
fn migrating_twice_is_a_no_op_the_second_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let mut first = Connection::open(&path).unwrap();
    let mut second = Connection::open(&path).unwrap();
    migrate_to_v1(&mut first).unwrap();
    // Both opens read version 1 before either migrated; the loser must not fail on CREATE TABLE.
    migrate_to_v2(&mut first).unwrap();
    migrate_to_v2(&mut second).unwrap();
    assert_eq!(user_version(&second), 2);
}

#[test]
fn a_wal_mode_v1_file_with_data_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let mut conn = Connection::open(&path).unwrap();
        migrate_to_v1(&mut conn).unwrap();
        let mode: String = conn
            .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        conn.execute_batch(
            "INSERT INTO series (id, connection, host, db, collection, index_name, definition_hash)
                 VALUES (1, 'c', 'h', 'shop', 'orders', 'by_sku', 'x');
             INSERT INTO samples (series_id, at_secs, ops, since_ms) VALUES (1, 100, 5, 1000);",
        )
        .unwrap();
    }
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert!(recovery.is_none());
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), 3);
    let samples: i64 = conn
        .query_row("SELECT COUNT(*) FROM samples", [], |r| r.get(0))
        .unwrap();
    assert_eq!(samples, 1);
    assert!(tables(&conn).contains(&"shape_bucket".to_owned()));
}
