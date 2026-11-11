//! Schema version 3: the shell tab's run history.
//!
//! One table, `shell_history`: the script text that actually ran (the selection or the whole
//! script), when, and how it ended. Never the output. Kept per connection id, newest 200 only.
//! The script is stored as written, so a secret typed into one is stored too; the file is 0600.
//!
//! Self-contained like [`super::shapes`]: the functions take the connection the caller already
//! locked, and `HistoryDb` adds the purged check around them.

use super::db::sql_err;
use crate::error::Result;
use crate::shell::Outcome;
use rusqlite::{Connection, params};
use std::fmt;
use uuid::Uuid;

/// Runs kept per connection; older ones are deleted as new ones arrive.
pub const SHELL_HISTORY_LIMIT: usize = 200;

/// Longest script (in bytes) that is stored. A longer one is run but not remembered. Kept small
/// because shell history counts toward the history file's size cap.
pub const MAX_STORED_SCRIPT_BYTES: usize = 32 * 1024;

/// The newest runs of each connection the size cap never deletes. Past these, the cap trims
/// shell history (oldest first, across all connections) before any usage or shape row.
pub const CAP_KEEP_PER_CONNECTION: usize = 20;

pub(super) const V3: &str = "
CREATE TABLE shell_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection TEXT NOT NULL,
    ran_at_secs INTEGER NOT NULL,
    script TEXT NOT NULL,
    status TEXT NOT NULL
);
CREATE INDEX shell_history_by_connection ON shell_history (connection, id);
";

/// How a recorded run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellRunStatus {
    Succeeded,
    ScriptErrors,
    Incomplete,
    Cancelled,
    CapReached,
    Failed,
}

impl ShellRunStatus {
    /// The stable text stored in the file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::ScriptErrors => "script_errors",
            Self::Incomplete => "incomplete",
            Self::Cancelled => "cancelled",
            Self::CapReached => "cap_reached",
            Self::Failed => "failed",
        }
    }

    /// The inverse of [`Self::as_str`]; `None` for text a newer build may have written.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "succeeded" => Self::Succeeded,
            "script_errors" => Self::ScriptErrors,
            "incomplete" => Self::Incomplete,
            "cancelled" => Self::Cancelled,
            "cap_reached" => Self::CapReached,
            "failed" => Self::Failed,
            _ => return None,
        })
    }

    /// The status to record for a finished run, or `None` when the script never executed
    /// (the shell did not connect or did not start), which is not recorded.
    pub fn from_outcome(outcome: &Outcome) -> Option<Self> {
        match outcome {
            Outcome::Exited(0) => Some(Self::Succeeded),
            Outcome::Exited(_) | Outcome::Signalled => Some(Self::Failed),
            Outcome::ScriptErrors => Some(Self::ScriptErrors),
            Outcome::Incomplete => Some(Self::Incomplete),
            Outcome::Cancelled => Some(Self::Cancelled),
            Outcome::CapReached => Some(Self::CapReached),
            Outcome::ConnectFailed | Outcome::StartFailed => None,
        }
    }
}

/// What recording a run did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// The run is in the history.
    Stored,
    /// The script is longer than [`MAX_STORED_SCRIPT_BYTES`], so it was not stored.
    TooLarge,
    /// Nothing to record: the connection was deleted, or the script never executed.
    Skipped,
}

/// One remembered run.
#[derive(Clone, PartialEq)]
pub struct ShellHistoryEntry {
    pub id: i64,
    pub ran_at_secs: i64,
    pub script: String,
    pub status: ShellRunStatus,
}

/// Leaves the script out: it may hold a secret the person typed, and events that carry an entry
/// are logged with `Debug`.
impl fmt::Debug for ShellHistoryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShellHistoryEntry")
            .field("id", &self.id)
            .field("ran_at_secs", &self.ran_at_secs)
            .field("script_bytes", &self.script.len())
            .field("status", &self.status)
            .finish()
    }
}

/// Trims `connection` to its newest [`SHELL_HISTORY_LIMIT`] rows.
fn trim(conn: &Connection, connection: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM shell_history WHERE connection = ?1 AND id NOT IN
             (SELECT id FROM shell_history WHERE connection = ?1 ORDER BY id DESC LIMIT ?2)",
        params![connection, SHELL_HISTORY_LIMIT as i64],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Deletes up to `limit` of the oldest runs across all connections, never one of the newest
/// [`CAP_KEEP_PER_CONNECTION`] of its connection; how many went. For the size cap.
pub(super) fn cap_batch(conn: &Connection, limit: i64) -> Result<usize> {
    conn.execute(
        "DELETE FROM shell_history WHERE id IN (
             SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (PARTITION BY connection ORDER BY id DESC) AS newer
                 FROM shell_history)
             WHERE newer > ?2 ORDER BY id LIMIT ?1)",
        params![limit, CAP_KEEP_PER_CONNECTION as i64],
    )
    .map_err(sql_err)
}

/// Stores one run: replaces the newest row when it holds the same script, then trims. One
/// transaction. A script over [`MAX_STORED_SCRIPT_BYTES`] is not stored ([`Recorded::TooLarge`]).
pub(super) fn record(
    conn: &mut Connection,
    connection: Uuid,
    script: &str,
    status: ShellRunStatus,
    now_secs: i64,
) -> Result<Recorded> {
    if script.len() > MAX_STORED_SCRIPT_BYTES {
        return Ok(Recorded::TooLarge);
    }
    let id = connection.to_string();
    let tx = conn.transaction().map_err(sql_err)?;
    tx.execute(
        "DELETE FROM shell_history WHERE script = ?2 AND id =
             (SELECT MAX(id) FROM shell_history WHERE connection = ?1)",
        params![id, script],
    )
    .map_err(sql_err)?;
    tx.execute(
        "INSERT INTO shell_history (connection, ran_at_secs, script, status)
         VALUES (?1, ?2, ?3, ?4)",
        params![id, now_secs, script, status.as_str()],
    )
    .map_err(sql_err)?;
    trim(&tx, &id)?;
    tx.commit().map_err(sql_err)?;
    Ok(Recorded::Stored)
}

/// The newest [`SHELL_HISTORY_LIMIT`] runs of a connection, newest first. A row whose status this
/// build does not know (written by a newer one) is left out.
pub(super) fn read(conn: &Connection, connection: Uuid) -> Result<Vec<ShellHistoryEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, ran_at_secs, script, status FROM shell_history
             WHERE connection = ?1 ORDER BY id DESC LIMIT ?2",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(
            params![connection.to_string(), SHELL_HISTORY_LIMIT as i64],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(sql_err)?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, ran_at_secs, script, status) = row.map_err(sql_err)?;
        if let Some(status) = ShellRunStatus::parse(&status) {
            entries.push(ShellHistoryEntry {
                id,
                ran_at_secs,
                script,
                status,
            });
        }
    }
    Ok(entries)
}

/// Deletes every run of a connection (the caller holds the connection lock): a clear, or a
/// purge.
pub(super) fn delete_connection(conn: &Connection, connection: Uuid) -> Result<()> {
    conn.execute(
        "DELETE FROM shell_history WHERE connection = ?1",
        [connection.to_string()],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Copies the newest runs of `from` to `to`, keeping their order, times and statuses, then trims
/// `to`. One transaction. Copying a connection onto itself does nothing.
pub(super) fn copy(conn: &mut Connection, from: Uuid, to: Uuid) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let tx = conn.transaction().map_err(sql_err)?;
    tx.execute(
        "INSERT INTO shell_history (connection, ran_at_secs, script, status)
         SELECT ?2, ran_at_secs, script, status FROM
             (SELECT id, ran_at_secs, script, status FROM shell_history
              WHERE connection = ?1 ORDER BY id DESC LIMIT ?3)
         ORDER BY id ASC",
        params![from.to_string(), to.to_string(), SHELL_HISTORY_LIMIT as i64],
    )
    .map_err(sql_err)?;
    trim(&tx, &to.to_string())?;
    tx.commit().map_err(sql_err)
}
