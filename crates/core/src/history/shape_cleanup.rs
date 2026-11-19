//! A one-time cleanup of recorded query shapes that no longer validate.
//!
//! The shape normalizer keeps only known query operators, so a row an older build wrote with some
//! other `$`-key (which may have been data) is rejected by `Shape::from_json` today. The cleanup
//! deletes such rows (their buckets cascade) once per file. A `shape_cleanup` marker row, written
//! in the same transaction as the deletes, says the rule ran, so a later open does not scan and
//! the schema version does not change. It never stores or logs a shape: it logs a count only
//! (at `info`, which the app's logger hides below `CURLYBRACE_LOG=info`).
//!
//! Limits: it runs once per file, so a row an older build writes after this one has run is never
//! cleaned (only a build before the allowlist can write one, and the shape normalizer of this
//! build never does). It does not run in `HistoryDb::open`: the runtime starts it on the blocking
//! pool right after the open (`HistoryDb::clean_shapes_once`), so startup never waits for it. It
//! waits only 250 ms for a write lock held elsewhere and then skips until the next start.

use rusqlite::types::Value;
use rusqlite::{Connection, ErrorCode, Transaction, TransactionBehavior};

use super::db::{BUSY_TIMEOUT, HistoryDb};
use super::db_schema::SchemaError;
use crate::shape::Shape;

/// The marker of the "drop shapes that fail validation" rule.
const RULE: i64 = 1;

/// Rows read per step, so a large history is never held in memory whole.
const BATCH: i64 = 500;

/// How long the pass waits for a write lock held elsewhere before it gives up until the next run.
const CLEANUP_BUSY_WAIT: std::time::Duration = std::time::Duration::from_millis(250);

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

impl HistoryDb {
    /// Runs the cleanup on this history (see [`run_once`]), meant for a background task after
    /// startup: it holds the connection for its pass (2.3 s in a debug build for 100 000 rows, 10 000
    /// of them invalid), so a read made meanwhile waits for it, and one made before it started shows the
    /// rows as an older build wrote them (readers do not re-validate a stored shape; the Queries
    /// view just offers no suggestion for one that does not parse). Only a damaged file is an
    /// error.
    pub fn clean_shapes_once(&self) -> crate::error::Result<()> {
        run_once(&mut self.lock())
            .map_err(|error| crate::error::CoreError::Database(format!("history: {error:?}")))
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
    // The pass holds the history's one connection: a lock another process keeps must not make
    // every reader wait the normal five seconds, so it waits briefly and tries again next time.
    conn.busy_timeout(CLEANUP_BUSY_WAIT)?;
    let result = sweep(conn);
    conn.busy_timeout(BUSY_TIMEOUT)?;
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
        checkpoint(conn);
    }
    Ok(())
}

/// Empties the write-ahead log so the deleted shapes are not left in uncheckpointed frames
/// (`secure_delete` zeroes pages of the database file, not frames already in the log). Cheap, as
/// the log of a history file is small and this runs only when something was deleted; in a file
/// that is not in WAL mode it does nothing. A busy file just keeps its log until the next
/// checkpoint, so a failure is logged and ignored.
fn checkpoint(conn: &Connection) {
    if let Err(error) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
        log::warn!(
            "history shape cleanup: checkpoint skipped: {:?}",
            error.sqlite_error_code()
        );
    }
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
