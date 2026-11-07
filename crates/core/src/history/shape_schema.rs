//! Schema version 2: query shape history (release B).
//!
//! Tables:
//! - `shape`: one row per (connection, host, db, collection, shape hash, source). `source` is
//!   `app` (seen in the profiler output of a node, `host` = that node) or `typed` (a query typed
//!   in the app, `host` = `""`). `shape_json` is the value-free text of `core::shape::Shape`;
//!   no literal value is ever stored.
//! - `shape_bucket`: additive counters per shape and bucket. `tier` 0 = raw 10-minute bucket,
//!   1 = hourly, 2 = daily (the same tier numbers and retention as `rolled`). `worst` is the
//!   stable `VerdictKind::code()`, ordered by `VerdictKind::severity`.
//! - `shape_gap`: seconds in which the profile poller may have missed entries of a database,
//!   per (connection, host, db) and the same tiers. Folded into every bucket of that database
//!   when read.

use crate::explain::VerdictKind;

/// Width of a raw (tier 0) shape bucket.
pub const RAW_BUCKET_SECS: i64 = 600;

/// Tier of a raw shape bucket.
pub const TIER_RAW: i64 = 0;

/// Largest value any shape counter may hold (2^43 - 1). Incoming values are clamped to it and
/// every accumulation is `MIN(a + b, ceiling)`, so a stored value never exceeds it. A rolling
/// `SUM` adds at most one row per distinct start second of one bucket width (at most 86 400,
/// under 2^17 rows) of values at most 2^43, which stays under 2^60 and cannot overflow `i64`.
/// Without this a hostile or corrupt count could turn a sum into a REAL or raise "integer
/// overflow", aborting the thinning (and the size cap with it) for the whole file.
pub const COUNT_CEILING: i64 = i64::MAX >> 20;

/// Host recorded for `typed` shapes: they come from the app, not from a node.
pub const TYPED_HOST: &str = "";

pub(super) const V2: &str = "
CREATE TABLE shape (
    id INTEGER PRIMARY KEY,
    connection TEXT NOT NULL,
    host TEXT NOT NULL,
    db TEXT NOT NULL,
    collection TEXT NOT NULL,
    hash TEXT NOT NULL,
    source TEXT NOT NULL,
    shape_json TEXT NOT NULL,
    UNIQUE (connection, host, db, collection, hash, source)
);
CREATE INDEX shape_by_collection ON shape (connection, db, collection);
CREATE TABLE shape_bucket (
    shape_id INTEGER NOT NULL REFERENCES shape (id) ON DELETE CASCADE,
    tier INTEGER NOT NULL,
    start_secs INTEGER NOT NULL,
    executions INTEGER NOT NULL,
    misses INTEGER NOT NULL,
    total_millis INTEGER NOT NULL,
    max_millis INTEGER NOT NULL,
    worst TEXT NOT NULL,
    gap_secs INTEGER NOT NULL,
    PRIMARY KEY (shape_id, tier, start_secs)
) WITHOUT ROWID;
CREATE INDEX shape_bucket_by_start ON shape_bucket (tier, start_secs);
CREATE TABLE shape_gap (
    connection TEXT NOT NULL,
    host TEXT NOT NULL,
    db TEXT NOT NULL,
    tier INTEGER NOT NULL,
    start_secs INTEGER NOT NULL,
    gap_secs INTEGER NOT NULL,
    PRIMARY KEY (connection, host, db, tier, start_secs)
) WITHOUT ROWID;
CREATE INDEX shape_gap_by_db ON shape_gap (connection, db, start_secs);
";

/// A SQL expression giving the severity of the verdict code in column `column`, built from
/// [`VerdictKind::ALL`] so it cannot drift from [`VerdictKind::severity`]. An unknown code ranks
/// 0 (neutral). Lets an upsert keep the worse of the stored and the incoming verdict in SQL.
pub(super) fn severity_sql(column: &str) -> String {
    let arms: String = VerdictKind::ALL
        .iter()
        .map(|k| format!(" WHEN '{}' THEN {}", k.code(), k.severity()))
        .collect();
    format!("(CASE {column}{arms} ELSE 0 END)")
}
