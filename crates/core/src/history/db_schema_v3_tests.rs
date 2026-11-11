//! The v2 to v3 migration: the shell history table appears, nothing else is touched.

use super::db::HistoryDb;
use super::db_schema::{SCHEMA_VERSION, migrate_to_v1, migrate_to_v2, migrate_to_v3};
use rusqlite::Connection;

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn has_table(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

#[test]
fn a_version_2_file_migrates_to_3_keeping_series_and_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let mut conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate_to_v1(&mut conn).unwrap();
        migrate_to_v2(&mut conn).unwrap();
        assert_eq!(user_version(&conn), 2);
        assert!(!has_table(&conn, "shell_history"));
        conn.execute_batch(
            "INSERT INTO series (id, connection, host, db, collection, index_name, definition_hash)
                 VALUES (1, 'c', 'h', 'shop', 'orders', 'by_sku', 'x');
             INSERT INTO samples (series_id, at_secs, ops, since_ms) VALUES (1, 100, 5, 1000);
             INSERT INTO shape (id, connection, host, db, collection, hash, source, shape_json)
                 VALUES (1, 'c', 'h', 'shop', 'orders', 'abc', 'app', '{}');
             INSERT INTO shape_bucket
                 (shape_id, tier, start_secs, executions, misses, total_millis, max_millis,
                  worst, gap_secs)
                 VALUES (1, 0, 600, 3, 1, 30, 20, 'collscan', 0);",
        )
        .unwrap();
    }
    let (db, recovery) = HistoryDb::open(&path).unwrap();
    assert!(recovery.is_none());
    drop(db);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(user_version(&conn), SCHEMA_VERSION);
    assert_eq!(SCHEMA_VERSION, 3);
    assert!(has_table(&conn, "shell_history"));
    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count("series"), 1);
    assert_eq!(count("samples"), 1);
    assert_eq!(count("shape"), 1);
    assert_eq!(count("shape_bucket"), 1);
    assert_eq!(count("shell_history"), 0);
}

#[test]
fn a_version_4_file_is_too_new() {
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
fn opening_twice_migrates_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let mut conn = Connection::open(&path).unwrap();
        migrate_to_v1(&mut conn).unwrap();
        migrate_to_v2(&mut conn).unwrap();
    }
    let mut first = Connection::open(&path).unwrap();
    let mut second = Connection::open(&path).unwrap();
    // Both saw version 2 before either migrated; the loser must not fail on CREATE TABLE.
    migrate_to_v3(&mut first).unwrap();
    migrate_to_v3(&mut second).unwrap();
    assert_eq!(user_version(&second), 3);
    let (db, _) = HistoryDb::open(&path).unwrap();
    drop(db);
    assert_eq!(user_version(&Connection::open(&path).unwrap()), 3);
}
