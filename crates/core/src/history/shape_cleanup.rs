//! A one-time cleanup of recorded query shapes that no longer validate.
//!
//! The shape normalizer keeps only known query operators, so a row an older build wrote with some
//! other `$`-key (which may have been data) is rejected by `Shape::from_json` today. The cleanup
//! deletes such rows (their buckets cascade) once per file. A `shape_cleanup` marker row, written
//! in the same transaction as the deletes, says the rule ran, so a later open does not scan and
//! the schema version does not change. It never stores or logs a shape: it logs a count only.

use rusqlite::types::Value;
use rusqlite::{Connection, ErrorCode, Transaction, TransactionBehavior};

use super::db_schema::SchemaError;
use crate::shape::Shape;

/// The marker of the "drop shapes that fail validation" rule.
const RULE: i64 = 1;

/// Rows read per step, so a large history is never held in memory whole.
const BATCH: i64 = 500;

const CREATE_MARKER: &str = "CREATE TABLE IF NOT EXISTS shape_cleanup (rule INTEGER PRIMARY KEY)";

/// Deletes `shape` rows (buckets cascade) whose `shape_json` `Shape::from_json` rejects, once per
/// file: the marker row for rule 1 is written in the same IMMEDIATE transaction. A damaged file
/// is reported so the open can set it aside; any other failure (a read-only file, a busy lock, a
/// full disk) skips the cleanup for this open, and the next writable open does it.
pub(super) fn run_once(conn: &mut Connection) -> Result<(), SchemaError> {
    match cleanup(conn) {
        Ok(()) => Ok(()),
        Err(error) if is_damage(&error) => Err(SchemaError::Sqlite(error)),
        Err(error) => {
            log::warn!(
                "history shape cleanup skipped: {:?}",
                error.sqlite_error_code()
            );
            Ok(())
        }
    }
}

fn is_damage(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
    )
}

/// Whether the marker row for [`RULE`] is there (a file without the marker table has not run).
fn is_done(conn: &Connection) -> rusqlite::Result<bool> {
    let table: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'shape_cleanup'",
        [],
        |r| r.get(0),
    )?;
    if table == 0 {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM shape_cleanup WHERE rule = ?1)",
        [RULE],
        |r| r.get(0),
    )
}

fn cleanup(conn: &mut Connection) -> rusqlite::Result<()> {
    if is_done(conn)? {
        return Ok(());
    }
    // Deleted shapes are zeroed on their pages, not left behind in the file (0600 or not).
    let secure: i64 = conn.pragma_query_value(None, "secure_delete", |r| r.get(0))?;
    conn.pragma_update(None, "secure_delete", true)?;
    let result = sweep(conn);
    if let Err(error) = conn.pragma_update(None, "secure_delete", secure) {
        log::warn!(
            "history secure_delete not restored: {:?}",
            error.sqlite_error_code()
        );
    }
    result
}

fn sweep(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(CREATE_MARKER)?;
    // Another open may have finished while this one waited for the lock.
    if is_done(&tx)? {
        return Ok(());
    }
    let removed = delete_invalid(&tx)?;
    tx.execute(
        "INSERT OR IGNORE INTO shape_cleanup (rule) VALUES (?1)",
        [RULE],
    )?;
    tx.commit()?;
    if removed > 0 {
        log::info!("removed {removed} recorded query shapes that no longer validate");
    }
    Ok(())
}

/// Whether the stored `shape_json` is a shape this build accepts; a non-text value is not.
fn is_valid(json: &Value) -> bool {
    matches!(json, Value::Text(text) if Shape::from_json(text).is_ok())
}

/// Deletes the invalid rows in batches by id and returns how many went.
fn delete_invalid(tx: &Transaction) -> rusqlite::Result<u64> {
    let (mut last, mut removed) = (i64::MIN, 0);
    loop {
        let batch: Vec<(i64, Value)> = tx
            .prepare("SELECT id, shape_json FROM shape WHERE id > ?1 ORDER BY id LIMIT ?2")?
            .query_map(rusqlite::params![last, BATCH], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let Some((newest, _)) = batch.last() else {
            return Ok(removed);
        };
        last = *newest;
        for (id, _) in batch.iter().filter(|(_, json)| !is_valid(json)) {
            tx.execute("DELETE FROM shape WHERE id = ?1", [id])?;
            removed += 1;
        }
    }
}
