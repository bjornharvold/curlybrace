//! The history database's schema and its versioned migrations (`PRAGMA user_version`).
//!
//! Tables:
//! - `series`: one row per (connection, node, database, collection, index name, definition
//!   hash). `rolled_through` is the Unix second up to which raw samples have been rolled into
//!   hourly rows; everything the hourly rows hold lies before it.
//! - `samples`: raw readings. The autoincrement `id` is the insertion order, which is the order
//!   `derive` must see them in (it never re-sorts by timestamp).
//! - `rolled`: thinned rows, `tier` 1 = hourly, 2 = daily, holding exactly a `Bucket`'s fields.

use rusqlite::Connection;

/// The schema version this build writes.
pub const SCHEMA_VERSION: i64 = 1;

/// Tier of an hourly rolled row.
pub const TIER_HOURLY: i64 = 1;

/// Tier of a daily rolled row.
pub const TIER_DAILY: i64 = 2;

/// Why preparing the database failed.
#[derive(Debug)]
pub enum SchemaError {
    Sqlite(rusqlite::Error),
    /// The file was written by a newer build; it is left alone.
    TooNew(i64),
}

impl From<rusqlite::Error> for SchemaError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

const V1: &str = "
CREATE TABLE series (
    id INTEGER PRIMARY KEY,
    connection TEXT NOT NULL,
    host TEXT NOT NULL,
    db TEXT NOT NULL,
    collection TEXT NOT NULL,
    index_name TEXT NOT NULL,
    definition_hash TEXT NOT NULL,
    rolled_through INTEGER NOT NULL DEFAULT 0,
    UNIQUE (connection, host, db, collection, index_name, definition_hash)
);
CREATE INDEX series_by_collection ON series (connection, db, collection);
CREATE TABLE samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    series_id INTEGER NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    at_secs INTEGER NOT NULL,
    ops INTEGER NOT NULL,
    since_ms INTEGER NOT NULL
);
CREATE INDEX samples_by_series ON samples (series_id, id);
CREATE TABLE rolled (
    series_id INTEGER NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    tier INTEGER NOT NULL,
    start_secs INTEGER NOT NULL,
    uses INTEGER NOT NULL,
    restarts INTEGER NOT NULL,
    gap_secs INTEGER NOT NULL,
    PRIMARY KEY (series_id, tier, start_secs)
) WITHOUT ROWID;
";

/// Applies the per-connection settings, then brings the schema up to [`SCHEMA_VERSION`].
pub fn prepare(conn: &mut Connection) -> Result<(), SchemaError> {
    conn.pragma_update(None, "foreign_keys", true)?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(SchemaError::TooNew(version));
    }
    if version < 1 {
        migrate_to_v1(conn)?;
    }
    Ok(())
}

fn migrate_to_v1(conn: &mut Connection) -> Result<(), SchemaError> {
    // Only takes effect before the first table exists: lets deleted rows give pages back.
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    let tx = conn.transaction()?;
    tx.execute_batch(V1)?;
    tx.pragma_update(None, "user_version", 1)?;
    tx.commit()?;
    Ok(())
}
