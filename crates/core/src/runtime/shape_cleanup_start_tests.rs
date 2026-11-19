//! The one-time shape cleanup runs in the background after the runtime starts: starting never
//! waits for it, even when another process holds the history file.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::*;
use crate::history::HistoryDb;

const INSERT_OLD: &str = "INSERT INTO shape (id, connection, host, db, collection, hash, source, shape_json)
    VALUES (1, 'c', 'h', 'shop', 'orders', 'h1', 'app', '{\"filter\":{\"a\":{\"$secretkey\":\"?\"}}}')";

fn shapes_left(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM shape", [], |r| r.get(0))
        .unwrap()
}

#[tokio::test]
async fn starting_the_runtime_does_not_wait_for_the_cleanup_and_the_cleanup_follows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    {
        let (db, _) = HistoryDb::open(&path).unwrap();
        db.exec_for_tests(INSERT_OLD);
    }
    // Another process holds the write lock while the runtime starts.
    let holder = Connection::open(&path).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let config = RuntimeConfig {
        history_path: Some(path.clone()),
        ..RuntimeConfig::default()
    };
    let started = Instant::now();
    let _handle = spawn_runtime_with_config(None, None, Arc::new(|| {}), config);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the runtime waited {:?}",
        started.elapsed()
    );
    holder.execute_batch("ROLLBACK").unwrap();
    drop(holder);
    // The background pass skipped while locked; a later open (here a second runtime) finishes it.
    let config = RuntimeConfig {
        history_path: Some(path.clone()),
        ..RuntimeConfig::default()
    };
    let _second = spawn_runtime_with_config(None, None, Arc::new(|| {}), config);
    for _ in 0..100 {
        if shapes_left(&path) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the cleanup never ran");
}
