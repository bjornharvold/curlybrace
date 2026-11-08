//! Query shape history (release B): recording what the profiler and the query editor ran.
//!
//! Only value-free data is stored: the `Shape` JSON text, its hash, names, counts and
//! durations. Buckets are additive, so recording, rolling (see [`super::shape_thin`]) and
//! reading are all sums, maxima and severity-maxima.
//!
//! `typed` rows (queries typed in the app) have no node: their host is
//! [`super::shape_schema::TYPED_HOST`] (`""`). `app` rows keep the host they were seen on.
//! The `worst` column holds `VerdictKind::code()`; the stored-vs-incoming comparison runs in SQL
//! through a `CASE` generated from `VerdictKind::ALL` (see `shape_schema::severity_sql`), and the
//! read side folds in Rust with `explain::worst`.

use super::db::{HistoryDb, sql_err};
use super::shape_schema::{COUNT_CEILING, RAW_BUCKET_SECS, TIER_RAW, TYPED_HOST, severity_sql};
use crate::error::Result;
use crate::explain::VerdictKind;
use rusqlite::{Connection, params};
use std::sync::LazyLock;
use uuid::Uuid;

/// Where a shape was observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeSource {
    /// Seen in a node's profiler output.
    App,
    /// A query typed in the app.
    Typed,
}

impl ShapeSource {
    /// The text stored in the `source` column.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Typed => "typed",
        }
    }
}

/// The text is not a stored [`ShapeSource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownShapeSource;

impl std::str::FromStr for ShapeSource {
    type Err = UnknownShapeSource;

    /// The inverse of [`ShapeSource::as_str`].
    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        match text {
            "app" => Ok(Self::App),
            "typed" => Ok(Self::Typed),
            _ => Err(UnknownShapeSource),
        }
    }
}

/// Identifies one stored shape. The same shape from another source or host is another row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShapeKey {
    pub connection: Uuid,
    pub host: String,
    pub db: String,
    pub collection: String,
    pub hash: String,
    pub source: ShapeSource,
}

/// What was seen of one shape in (part of) one bucket.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeObservation {
    pub key: ShapeKey,
    /// The value-free `Shape::json()` text.
    pub shape_json: String,
    pub at_secs: i64,
    pub executions: u64,
    pub misses: u64,
    pub total_millis: u64,
    pub max_millis: u64,
    pub worst: VerdictKind,
    pub gap_secs: u64,
}

/// One bucket of a shape's chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeBucket {
    pub start_secs: i64,
    pub width_secs: i64,
    pub executions: u64,
    pub misses: u64,
    /// Seconds of the bucket in which entries may have been missed (never more than the width).
    pub gap_secs: u64,
}

/// A shape's totals over a chart range, and its buckets.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeSummary {
    pub hash: String,
    pub shape_json: String,
    pub source: ShapeSource,
    pub executions: u64,
    pub misses: u64,
    pub total_millis: u64,
    pub max_millis: u64,
    /// The worst verdict of the whole range.
    pub worst: VerdictKind,
    /// The verdict of the newest bucket that ran the shape: what it does now. After an index is
    /// created this stops being a miss while `worst` still remembers the scans.
    pub latest: VerdictKind,
    pub buckets: Vec<ShapeBucket>,
}

/// Whether an observation with this verdict counts as a miss: an index could have served the
/// query better. Neutral (`NoFilter`, `EmptyCollection`) and good verdicts are not misses.
pub fn is_miss(kind: VerdictKind) -> bool {
    matches!(
        kind,
        VerdictKind::CollectionScan | VerdictKind::SortNotCovered | VerdictKind::InefficientIndex
    )
}

/// The counters of one bucket row, as stored.
pub(super) struct BucketValues<'a> {
    pub executions: i64,
    pub misses: i64,
    pub total_millis: i64,
    pub max_millis: i64,
    pub worst: &'a str,
    pub gap_secs: i64,
}

static UPSERT_BUCKET: LazyLock<String> = LazyLock::new(|| {
    format!(
        "INSERT INTO shape_bucket
             (shape_id, tier, start_secs, executions, misses, total_millis, max_millis, worst,
              gap_secs)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (shape_id, tier, start_secs) DO UPDATE SET
             executions = MIN(executions + excluded.executions, {ceiling}),
             misses = MIN(misses + excluded.misses, {ceiling}),
             total_millis = MIN(total_millis + excluded.total_millis, {ceiling}),
             max_millis = MAX(max_millis, excluded.max_millis),
             worst = CASE WHEN {incoming} > {stored} THEN excluded.worst ELSE worst END,
             gap_secs = MIN(gap_secs + excluded.gap_secs, {ceiling})",
        ceiling = COUNT_CEILING,
        incoming = severity_sql("excluded.worst"),
        stored = severity_sql("shape_bucket.worst"),
    )
});

/// Adds `values` to a bucket row, creating it when absent.
pub(super) fn add_bucket(
    conn: &Connection,
    shape_id: i64,
    tier: i64,
    start_secs: i64,
    values: &BucketValues,
) -> Result<()> {
    conn.prepare_cached(&UPSERT_BUCKET)
        .and_then(|mut stmt| {
            stmt.execute(params![
                shape_id,
                tier,
                start_secs,
                values.executions,
                values.misses,
                values.total_millis,
                values.max_millis,
                values.worst,
                values.gap_secs
            ])
        })
        .map(drop)
        .map_err(sql_err)
}

/// Adds seconds to a gap row, creating it when absent.
pub(super) fn add_gap(
    conn: &Connection,
    (connection, host, db): (&str, &str, &str),
    tier: i64,
    start_secs: i64,
    gap_secs: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO shape_gap (connection, host, db, tier, start_secs, gap_secs)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (connection, host, db, tier, start_secs) DO UPDATE SET
             gap_secs = MIN(gap_secs + excluded.gap_secs, ?7)",
        params![
            connection,
            host,
            db,
            tier,
            start_secs,
            gap_secs,
            COUNT_CEILING
        ],
    )
    .map(drop)
    .map_err(sql_err)
}

/// A count clamped to [`COUNT_CEILING`], so sums of stored values cannot overflow.
pub(super) fn to_sql(n: u64) -> i64 {
    i64::try_from(n).map_or(COUNT_CEILING, |v| v.min(COUNT_CEILING))
}

fn shape_id(conn: &Connection, row: &ShapeObservation) -> Result<i64> {
    let key = &row.key;
    // A typed query belongs to no node: its host is always empty, whatever the caller passed.
    let host = match key.source {
        ShapeSource::App => key.host.as_str(),
        ShapeSource::Typed => TYPED_HOST,
    };
    conn.query_row(
        "INSERT INTO shape (connection, host, db, collection, hash, source, shape_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (connection, host, db, collection, hash, source)
         DO UPDATE SET id = id
         RETURNING id",
        params![
            key.connection.to_string(),
            host,
            key.db,
            key.collection,
            key.hash,
            key.source.as_str(),
            row.shape_json
        ],
        |r| r.get(0),
    )
    .map_err(sql_err)
}

impl HistoryDb {
    /// Adds observations to their shapes' raw 10-minute buckets, in one transaction. Rows of a
    /// purged connection are skipped, checked under the connection lock like [`Self::record_many`].
    pub fn record_shapes(&self, rows: &[ShapeObservation]) -> Result<()> {
        let mut conn = self.lock();
        let tx = conn.transaction().map_err(sql_err)?;
        self.insert_observations(&tx, rows)?;
        tx.commit().map_err(sql_err)
    }

    /// Adds the observations of connections not purged (the caller holds the connection lock
    /// and owns the transaction).
    pub(super) fn insert_observations(
        &self,
        conn: &Connection,
        rows: &[ShapeObservation],
    ) -> Result<()> {
        for row in rows.iter().filter(|r| !self.is_purged(r.key.connection)) {
            let id = shape_id(conn, row)?;
            let values = BucketValues {
                executions: to_sql(row.executions),
                misses: to_sql(row.misses),
                total_millis: to_sql(row.total_millis),
                max_millis: to_sql(row.max_millis),
                worst: row.worst.code(),
                gap_secs: to_sql(row.gap_secs),
            };
            let start = row.at_secs.div_euclid(RAW_BUCKET_SECS) * RAW_BUCKET_SECS;
            add_bucket(conn, id, TIER_RAW, start, &values)?;
        }
        Ok(())
    }

    /// Notes that the profile poller may have missed entries of `db` for `gap_secs` seconds
    /// around `at_secs`. Reads fold it into every shape of that database.
    pub fn record_gap(
        &self,
        connection: Uuid,
        host: &str,
        db: &str,
        at_secs: i64,
        gap_secs: u64,
    ) -> Result<()> {
        let conn = self.lock();
        if self.is_purged(connection) {
            return Ok(());
        }
        let start = at_secs.div_euclid(RAW_BUCKET_SECS) * RAW_BUCKET_SECS;
        let id = connection.to_string();
        add_gap(&conn, (&id, host, db), TIER_RAW, start, to_sql(gap_secs))
    }
}

impl HistoryDb {
    /// Notes that entries of `db` may have been missed from `from_secs` for `span_secs`, spread
    /// over the 10-minute buckets the span covers (the first and the last may be partial), in ONE
    /// transaction so a retry never counts a part of it twice. At most [`MAX_GAP_BUCKETS`]
    /// buckets are written: of a longer span only its newest part is kept.
    pub fn record_gap_span(
        &self,
        connection: Uuid,
        host: &str,
        db: &str,
        from_secs: i64,
        span_secs: u64,
    ) -> Result<()> {
        let span = i64::try_from(span_secs).unwrap_or(i64::MAX);
        let end = from_secs.saturating_add(span);
        let start = from_secs.max(end.saturating_sub(MAX_GAP_BUCKETS * RAW_BUCKET_SECS));
        let id = connection.to_string();
        let mut conn = self.lock();
        // Under the lock, like `record_gap`: a purge marks the connection while it holds it.
        if self.is_purged(connection) {
            return Ok(());
        }
        let tx = conn.transaction().map_err(sql_err)?;
        let mut at = start;
        while at < end {
            // Each part lies inside one bucket; the first and the last may be partial.
            let bucket = at.div_euclid(RAW_BUCKET_SECS) * RAW_BUCKET_SECS;
            let next = end.min(bucket + RAW_BUCKET_SECS);
            add_gap(&tx, (&id, host, db), TIER_RAW, bucket, next - at)?;
            at = next;
        }
        tx.commit().map_err(sql_err)
    }
}

/// Most buckets one lost span is spread over: the raw retention in buckets.
const MAX_GAP_BUCKETS: i64 = super::retention::RAW_RETENTION_SECS / RAW_BUCKET_SECS;

/// Deletes every shape row and gap of a connection (the caller holds the connection lock).
pub(super) fn delete_connection(conn: &Connection, connection: Uuid) -> Result<()> {
    let id = connection.to_string();
    for sql in [
        "DELETE FROM shape WHERE connection = ?1",
        "DELETE FROM shape_gap WHERE connection = ?1",
        "DELETE FROM profile_mark WHERE connection = ?1",
    ] {
        conn.execute(sql, [&id]).map_err(sql_err)?;
    }
    Ok(())
}
