//! Reading shape history for a chart: all tiers folded into buckets of the range's width.
//!
//! Tiers never overlap (rolling deletes what it folds, on whole-hour and whole-day cutoffs), so
//! the tiers are simply summed into the bucket each row's start falls in.
//!
//! ## Gap seconds
//!
//! A bucket's `gap_secs` is the seconds in which entries of its database may have been missed.
//! Per host it is the shape row's own gap plus the gap rows recorded for that database and host;
//! the bucket takes the MAX over hosts (a union upper bound: hosts are polled in the same
//! seconds, so they are never added together), clamped to the bucket width. Buckets where a
//! shape has no execution but its database has a gap are returned too (zero executions).

use super::db::{HistoryDb, sql_err};
use super::retention::ChartRange;
use super::shapes::{ShapeBucket, ShapeSource, ShapeSummary};
use crate::error::Result;
use crate::explain::{VerdictKind, worst};
use rusqlite::{Connection, Row, params};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Default)]
struct BucketAcc {
    executions: u64,
    misses: u64,
    /// Gap seconds by host (see the module docs).
    gap_by_host: BTreeMap<String, u64>,
}

struct SummaryAcc {
    shape_json: String,
    source: ShapeSource,
    executions: u64,
    misses: u64,
    total_millis: u64,
    max_millis: u64,
    worst: VerdictKind,
    /// Start and verdict of the newest bucket row that executed the shape (the worse of equal
    /// starts, e.g. two nodes).
    latest: Option<(i64, VerdictKind)>,
    buckets: BTreeMap<i64, BucketAcc>,
}

/// One stored bucket row, decoded.
struct RowData {
    hash: String,
    source: ShapeSource,
    shape_json: String,
    host: String,
    start_secs: i64,
    executions: u64,
    misses: u64,
    total_millis: u64,
    max_millis: u64,
    verdict: VerdictKind,
    gap_secs: u64,
}

impl SummaryAcc {
    fn new(row: &RowData) -> Self {
        Self {
            shape_json: row.shape_json.clone(),
            source: row.source,
            executions: 0,
            misses: 0,
            total_millis: 0,
            max_millis: 0,
            worst: VerdictKind::NoFilter,
            latest: None,
            buckets: BTreeMap::new(),
        }
    }

    fn add(&mut self, row: &RowData, width: i64) {
        self.executions = self.executions.saturating_add(row.executions);
        self.misses = self.misses.saturating_add(row.misses);
        self.total_millis = self.total_millis.saturating_add(row.total_millis);
        self.max_millis = self.max_millis.max(row.max_millis);
        self.worst = worst(self.worst, row.verdict);
        if row.executions > 0 {
            self.note_latest(row.start_secs, row.verdict);
        }
        let bucket = self
            .buckets
            .entry(bucket_start(row.start_secs, width))
            .or_default();
        bucket.executions = bucket.executions.saturating_add(row.executions);
        bucket.misses = bucket.misses.saturating_add(row.misses);
        let gap = bucket.gap_by_host.entry(row.host.clone()).or_default();
        *gap = gap.saturating_add(row.gap_secs);
    }
}

impl SummaryAcc {
    fn note_latest(&mut self, start_secs: i64, verdict: VerdictKind) {
        self.latest = match self.latest {
            Some((start, _)) if start > start_secs => self.latest,
            Some((start, seen)) if start == start_secs => Some((start, worst(seen, verdict))),
            _ => Some((start_secs, verdict)),
        };
    }
}

fn unsigned(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn bucket_start(start_secs: i64, width: i64) -> i64 {
    start_secs.div_euclid(width) * width
}

impl HistoryDb {
    /// Every shape stored for a collection, summed over `range` (the `range.secs()` before
    /// `now_secs`; rows stamped after `now_secs` are ignored) and bucketed
    /// `range.bucket_width_secs()` wide. Hosts are summed (grouped by hash and source); see the
    /// module docs for how gaps combine. Ordered by latest verdict, then misses, then slowest
    /// execution, all descending.
    pub fn shapes_for(
        &self,
        connection: Uuid,
        db: &str,
        collection: &str,
        range: ChartRange,
        now_secs: i64,
    ) -> Result<Vec<ShapeSummary>> {
        let conn = self.lock();
        let width = range.bucket_width_secs();
        let window = (bucket_start(now_secs - range.secs(), width), now_secs);
        let mut summaries = read_shapes(&conn, connection, (db, collection), window, width)?;
        for (host, start, gap) in read_gaps(&conn, connection, db, window)? {
            // Only shapes seen in the profile can have missed entries; a typed query never was.
            for summary in summaries
                .values_mut()
                .filter(|s| s.source == ShapeSource::App)
            {
                let bucket = summary
                    .buckets
                    .entry(bucket_start(start, width))
                    .or_default();
                let slot = bucket.gap_by_host.entry(host.clone()).or_default();
                *slot = slot.saturating_add(gap);
            }
        }
        let mut out: Vec<_> = summaries
            .into_iter()
            .map(|((hash, _), acc)| finish(hash, acc, width))
            .collect();
        out.sort_by(|a, b| {
            (b.latest.severity(), b.misses, b.max_millis, &a.hash).cmp(&(
                a.latest.severity(),
                a.misses,
                a.max_millis,
                &b.hash,
            ))
        });
        Ok(out)
    }
}

fn finish(hash: String, acc: SummaryAcc, width: i64) -> ShapeSummary {
    let buckets = acc
        .buckets
        .into_iter()
        .map(|(start_secs, b)| ShapeBucket {
            start_secs,
            width_secs: width,
            executions: b.executions,
            misses: b.misses,
            gap_secs: b
                .gap_by_host
                .values()
                .copied()
                .max()
                .unwrap_or(0)
                .min(unsigned(width)),
        })
        .collect();
    ShapeSummary {
        hash,
        shape_json: acc.shape_json,
        source: acc.source,
        executions: acc.executions,
        misses: acc.misses,
        total_millis: acc.total_millis,
        max_millis: acc.max_millis,
        worst: acc.worst,
        latest: acc.latest.map_or(acc.worst, |(_, verdict)| verdict),
        buckets,
    }
}

type Summaries = BTreeMap<(String, &'static str), SummaryAcc>;

/// Decodes a row; `unknown` counts values this build does not understand.
fn decode(r: &Row, unknown: &mut usize) -> Result<Option<RowData>> {
    let count = |i: usize| r.get::<_, i64>(i).map(unsigned).map_err(sql_err);
    let source_text: String = r.get(1).map_err(sql_err)?;
    let Ok(source) = source_text.parse::<ShapeSource>() else {
        *unknown += 1;
        return Ok(None);
    };
    let code: String = r.get(9).map_err(sql_err)?;
    let verdict = VerdictKind::from_code(&code).unwrap_or_else(|| {
        *unknown += 1;
        VerdictKind::NoFilter
    });
    Ok(Some(RowData {
        hash: r.get(0).map_err(sql_err)?,
        source,
        shape_json: r.get(2).map_err(sql_err)?,
        host: r.get(3).map_err(sql_err)?,
        start_secs: r.get(4).map_err(sql_err)?,
        executions: count(5)?,
        misses: count(6)?,
        total_millis: count(7)?,
        max_millis: count(8)?,
        verdict,
        gap_secs: count(10)?,
    }))
}

fn read_shapes(
    conn: &Connection,
    connection: Uuid,
    (db, collection): (&str, &str),
    (from, to): (i64, i64),
    width: i64,
) -> Result<Summaries> {
    let mut stmt = conn
        .prepare(
            "SELECT s.hash, s.source, s.shape_json, s.host, b.start_secs, b.executions,
                    b.misses, b.total_millis, b.max_millis, b.worst, b.gap_secs
             FROM shape s JOIN shape_bucket b ON b.shape_id = s.id
             WHERE s.connection = ?1 AND s.db = ?2 AND s.collection = ?3
               AND b.start_secs >= ?4 AND b.start_secs <= ?5",
        )
        .map_err(sql_err)?;
    let mut rows = stmt
        .query(params![connection.to_string(), db, collection, from, to])
        .map_err(sql_err)?;
    let mut out = Summaries::new();
    let mut unknown = 0_usize;
    while let Some(r) = rows.next().map_err(sql_err)? {
        let Some(row) = decode(r, &mut unknown)? else {
            continue;
        };
        out.entry((row.hash.clone(), row.source.as_str()))
            .or_insert_with(|| SummaryAcc::new(&row))
            .add(&row, width);
    }
    if unknown > 0 {
        log::warn!("history database: {unknown} shape value(s) not understood by this build");
    }
    Ok(out)
}

/// Gap rows of the database in the window: (host, start, seconds).
fn read_gaps(
    conn: &Connection,
    connection: Uuid,
    db: &str,
    (from, to): (i64, i64),
) -> Result<Vec<(String, i64, u64)>> {
    let mut stmt = conn
        .prepare(
            "SELECT host, start_secs, gap_secs FROM shape_gap
             WHERE connection = ?1 AND db = ?2 AND start_secs >= ?3 AND start_secs <= ?4",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![connection.to_string(), db, from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, unsigned(r.get(2)?)))
        })
        .map_err(sql_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_err)
}
