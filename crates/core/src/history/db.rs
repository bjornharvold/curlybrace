//! SQLite-backed store of `$indexStats` samples, keyed by series.
//!
//! One file serves every connection. Each method is a short blocking call behind a mutex; the
//! runtime calls them through `spawn_blocking`. Thinning lives in [`super::thin`], reading a
//! chart in [`super::usage`], the schema in [`super::db_schema`].

use super::db_schema::{SchemaError, prepare};
use super::retention::{ChartRange, MAX_DB_BYTES};
use super::series::{Bucket, Sample};
use super::{thin, usage};
use crate::error::{CoreError, Result};
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use uuid::Uuid;

/// Identifies one plotted series. A different definition hash (an index recreated with other
/// options) or a different host (another node) is a different series; they are never joined.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SeriesKey {
    pub connection: Uuid,
    pub host: String,
    pub db: String,
    pub collection: String,
    pub index_name: String,
    pub definition_hash: String,
}

/// What `open` did to get a usable database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// The file was unreadable or corrupt. It was moved to this path (never deleted) and a fresh
    /// database was created in its place.
    SetAside(PathBuf),
}

/// The sample store.
pub struct HistoryDb {
    pub(super) conn: Mutex<Connection>,
    max_bytes: u64,
}

pub(super) fn sql_err(e: rusqlite::Error) -> CoreError {
    CoreError::Database(format!("history: {e}"))
}

impl HistoryDb {
    /// Makes every later `record` and `thin` fail, as a damaged database would (tests only).
    #[cfg(test)]
    pub(crate) fn break_writes_for_tests(&self) {
        self.lock()
            .execute_batch("ALTER TABLE samples RENAME TO samples_gone")
            .expect("rename the samples table");
    }

    /// Opens (creating, with mode 0600 on unix) the database at `path`. A corrupt file is moved
    /// to `<name>.corrupt` and replaced; a file from a newer build is refused untouched.
    pub fn open(path: &Path) -> Result<(Self, Option<Recovery>)> {
        match open_file(path) {
            Ok(db) => Ok((db, None)),
            Err(OpenFailure::Corrupt(cause)) => {
                let aside = set_aside(path)?;
                log::warn!(
                    "history database unreadable ({cause}); moved to {}",
                    aside.display()
                );
                let db = open_file(path).map_err(OpenFailure::into_core)?;
                Ok((db, Some(Recovery::SetAside(aside))))
            }
            Err(other) => Err(other.into_core()),
        }
    }

    /// A private in-memory database, for tests and for running without a file.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory().map_err(sql_err)?;
        prepare(&mut conn).map_err(|e| OpenFailure::from(e).into_core())?;
        Ok(Self::from_connection(conn))
    }

    fn from_connection(conn: Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
            max_bytes: MAX_DB_BYTES,
        }
    }

    /// The same database with a different size cap, so tests need not write 100 MB.
    #[cfg(test)]
    pub(super) fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// The same database capped `delta` bytes below what it uses now.
    #[cfg(test)]
    pub(super) fn with_max_bytes_from_used(self, delta: u64) -> Self {
        let cap = self.used_bytes().unwrap() - delta;
        self.with_max_bytes(cap)
    }

    /// Bytes in use by live pages (free pages left behind by deletes do not count).
    #[cfg(test)]
    pub(super) fn used_bytes(&self) -> Result<u64> {
        thin::used_bytes(&self.lock())
    }

    /// A poisoned lock only means another call panicked; transactions roll back on drop, so the
    /// connection is still consistent.
    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends one reading to a series. Readings are kept in insertion order, which is the
    /// order they are read back in, whatever their timestamps say.
    pub fn record(&self, key: &SeriesKey, sample: Sample) -> Result<()> {
        self.record_many(&[(key.clone(), sample)])
    }

    /// Appends many readings in one transaction: one commit (one fsync) for the whole batch
    /// instead of one per series, and all or none of them are stored.
    pub fn record_many(&self, samples: &[(SeriesKey, Sample)]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(sql_err)?;
        for (key, sample) in samples {
            let series_id: i64 = tx
                .query_row(
                    "INSERT INTO series
                         (connection, host, db, collection, index_name, definition_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (connection, host, db, collection, index_name, definition_hash)
                     DO UPDATE SET host = excluded.host
                     RETURNING id",
                    params![
                        key.connection.to_string(),
                        key.host,
                        key.db,
                        key.collection,
                        key.index_name,
                        key.definition_hash
                    ],
                    |r| r.get(0),
                )
                .map_err(sql_err)?;
            tx.execute(
                "INSERT INTO samples (series_id, at_secs, ops, since_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![series_id, sample.at_secs, sample.ops, sample.since_ms],
            )
            .map_err(sql_err)?;
        }
        tx.commit().map_err(sql_err)
    }

    /// Buckets of the last `range.secs()` seconds before `now_secs`, `range.bucket_width_secs()`
    /// wide. See [`super::usage`] for how raw and rolled data are merged.
    pub fn usage(&self, key: &SeriesKey, range: ChartRange, now_secs: i64) -> Result<Vec<Bucket>> {
        usage::usage(&self.lock(), key, range, now_secs)
    }

    /// The most recently inserted sample of a series (not the one with the latest timestamp).
    pub fn latest(&self, key: &SeriesKey) -> Result<Option<Sample>> {
        let conn = self.lock();
        let Some((series_id, _)) = usage::find_series(&conn, key)? else {
            return Ok(None);
        };
        conn.query_row(
            "SELECT at_secs, ops, since_ms FROM samples WHERE series_id = ?1
             ORDER BY id DESC LIMIT 1",
            [series_id],
            |r| {
                Ok(Sample {
                    at_secs: r.get(0)?,
                    ops: r.get(1)?,
                    since_ms: r.get(2)?,
                })
            },
        )
        .optional()
        .map_err(sql_err)
    }

    /// Every series stored for a collection, across nodes and index definitions, including
    /// those of indexes that were since dropped (until retention removes them).
    pub fn series_for(
        &self,
        connection: Uuid,
        db: &str,
        collection: &str,
    ) -> Result<Vec<SeriesKey>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare(
                "SELECT host, index_name, definition_hash FROM series
                 WHERE connection = ?1 AND db = ?2 AND collection = ?3
                 ORDER BY index_name, definition_hash, host",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map(params![connection.to_string(), db, collection], |r| {
                Ok(SeriesKey {
                    connection,
                    host: r.get(0)?,
                    db: db.to_owned(),
                    collection: collection.to_owned(),
                    index_name: r.get(1)?,
                    definition_hash: r.get(2)?,
                })
            })
            .map_err(sql_err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql_err)
    }

    /// Deletes everything stored for one connection (its samples and rolled rows).
    pub fn purge_connection(&self, connection: Uuid) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "DELETE FROM series WHERE connection = ?1",
            [connection.to_string()],
        )
        .map_err(sql_err)?;
        thin::release_free_pages(&conn)
    }

    /// Rolls old data into coarser rows, drops expired rows, then enforces the size cap.
    pub fn thin(&self, now_secs: i64) -> Result<()> {
        thin::thin(&mut self.lock(), now_secs, self.max_bytes)
    }
}

/// Why `open_file` failed.
enum OpenFailure {
    /// SQLite says the file is not a database or is damaged.
    Corrupt(String),
    TooNew(i64),
    Other(CoreError),
}

impl OpenFailure {
    fn into_core(self) -> CoreError {
        match self {
            Self::Corrupt(cause) => CoreError::Database(format!("history: {cause}")),
            Self::TooNew(version) => CoreError::Unsupported(format!(
                "history database has schema version {version}, newer than this build"
            )),
            Self::Other(e) => e,
        }
    }
}

impl From<SchemaError> for OpenFailure {
    fn from(e: SchemaError) -> Self {
        match e {
            SchemaError::TooNew(v) => Self::TooNew(v),
            SchemaError::Sqlite(e) => Self::from(e),
        }
    }
}

impl From<rusqlite::Error> for OpenFailure {
    fn from(e: rusqlite::Error) -> Self {
        match e.sqlite_error_code() {
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
                Self::Corrupt(e.to_string())
            }
            _ => Self::Other(sql_err(e)),
        }
    }
}

fn io_err(e: std::io::Error) -> CoreError {
    CoreError::Database(format!("history file: {e}"))
}

fn open_file(path: &Path) -> std::result::Result<HistoryDb, OpenFailure> {
    create_private_file(path).map_err(|e| OpenFailure::Other(io_err(e)))?;
    let mut conn = Connection::open(path)?;
    prepare(&mut conn)?;
    Ok(HistoryDb::from_connection(conn))
}

/// Makes sure the file exists with mode 0600 before SQLite opens it, so it is never readable by
/// others, not even briefly.
fn create_private_file(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Moves a damaged database to `<name>.corrupt` (`.corrupt.1`, `.corrupt.2`, ... when taken, so
/// an earlier set-aside file is never overwritten).
fn set_aside(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| CoreError::Database("history file: path has no file name".into()))?
        .to_string_lossy()
        .into_owned();
    let target = (0..1_000)
        .map(|n| match n {
            0 => format!("{name}.corrupt"),
            n => format!("{name}.corrupt.{n}"),
        })
        .map(|candidate| path.with_file_name(candidate))
        .find(|candidate| !candidate.exists())
        .ok_or_else(|| CoreError::Database("history file: no free set-aside name".into()))?;
    std::fs::rename(path, &target).map_err(io_err)?;
    Ok(target)
}
