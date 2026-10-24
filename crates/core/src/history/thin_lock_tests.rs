//! Thinning reads before it writes. In WAL mode a commit by another connection between the two
//! fails a deferred transaction at once, so thinning takes the write lock when it begins.

use super::db::*;
use super::series::{DEFAULT_MAX_CONTINUOUS_SECS, Sample};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

const NOW: i64 = 1_800_000_000;
/// VM steps into a thinning after which the other connection tries to write: past the
/// transaction's start, well inside its reads.
const WRITE_AFTER_STEPS: u32 = 200;

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

/// A file database holding a few hundred raw samples that are old enough to roll.
fn old_history(dir: &tempfile::TempDir) -> (HistoryDb, std::path::PathBuf) {
    let path = dir.path().join("history.sqlite");
    let (db, _) = HistoryDb::open(&path).unwrap();
    let old = NOW - 100 * 3_600;
    let samples: Vec<_> = (0..300)
        .map(|n| {
            let sample = Sample {
                at_secs: old + n * 60,
                ops: n,
                since_ms: 1_000,
            };
            (key(), sample)
        })
        .collect();
    db.record_many(&samples).unwrap();
    (db, path)
}

#[test]
fn thinning_works_while_another_connection_holds_no_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, path) = old_history(&dir);
    let _idle = rusqlite::Connection::open(&path).unwrap();
    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();
}

#[test]
fn another_writer_cannot_slip_in_between_the_reads_and_the_writes_of_a_thinning() {
    let dir = tempfile::tempdir().unwrap();
    let (db, path) = old_history(&dir);
    let other = rusqlite::Connection::open(&path).unwrap();
    other.busy_timeout(Duration::ZERO).unwrap();
    let outcome: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&outcome);
    let mut steps = 0_u32;
    db.conn
        .lock()
        .unwrap()
        .progress_handler(
            1,
            Some(move || {
                steps += 1;
                if steps == WRITE_AFTER_STEPS {
                    let wrote = other.execute_batch("BEGIN IMMEDIATE; COMMIT;").is_ok();
                    *seen.lock().unwrap() = Some(wrote);
                }
                false
            }),
        )
        .unwrap();

    let thinned = db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS);

    assert_eq!(
        *outcome.lock().unwrap(),
        Some(false),
        "the thinning held the write lock from its start"
    );
    thinned.unwrap();
}
