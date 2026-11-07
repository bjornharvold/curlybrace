//! Reading shape history for a chart: all tiers folded into buckets of the range's width.
//!
//! Tiers never overlap (rolling deletes what it folds, on whole-hour and whole-day cutoffs), so
//! the tiers are simply summed into the bucket each row's start falls in. Gap seconds of the
//! database are added to every bucket, including buckets where the shape has no execution.

use super::db::{HistoryDb, sql_err};
use super::retention::ChartRange;
use super::shapes::{ShapeBucket, ShapeSource, ShapeSummary};
use crate::error::Result;
use crate::explain::{VerdictKind, worst};
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Default)]
struct BucketAcc {
    executions: u64,
    misses: u64,
    gap_secs: u64,
}

struct SummaryAcc {
    shape_json: String,
    source: ShapeSource,
    executions: u64,
    misses: u64,
    total_millis: u64,
    max_millis: u64,
    worst: VerdictKind,
    buckets: BTreeMap<i64, BucketAcc>,
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
    /// `range.bucket_width_secs()` wide. Hosts are summed (grouped by hash and source). Ordered
    /// by worst verdict, then misses, then slowest execution, all descending.
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
        let from = bucket_start(now_secs - range.secs(), width);
        let mut summaries =
            read_shapes(&conn, connection, (db, collection), (from, now_secs), width)?;
        for (start, gap) in read_gaps(&conn, connection, db, (from, now_secs), width)? {
            for summary in summaries.values_mut() {
                summary.buckets.entry(start).or_default().gap_secs += gap;
            }
        }
        let mut out: Vec<_> = summaries
            .into_iter()
            .map(|((hash, _), acc)| finish(hash, acc, width))
            .collect();
        out.sort_by(|a, b| {
            (b.worst.severity(), b.misses, b.max_millis, &a.hash).cmp(&(
                a.worst.severity(),
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
            gap_secs: b.gap_secs.min(unsigned(width)),
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
        buckets,
    }
}

type Summaries = BTreeMap<(String, &'static str), SummaryAcc>;

fn read_shapes(
    conn: &Connection,
    connection: Uuid,
    (db, collection): (&str, &str),
    (from, to): (i64, i64),
    width: i64,
) -> Result<Summaries> {
    let mut stmt = conn
        .prepare(
            "SELECT s.hash, s.source, s.shape_json, b.start_secs, b.executions, b.misses,
                    b.total_millis, b.max_millis, b.worst, b.gap_secs
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
        let get = |i: usize| r.get::<_, i64>(i).map(unsigned).map_err(sql_err);
        let source_text: String = r.get(1).map_err(sql_err)?;
        let Some(source) = ShapeSource::from_str(&source_text) else {
            unknown += 1;
            continue;
        };
        let code: String = r.get(8).map_err(sql_err)?;
        let verdict = VerdictKind::from_code(&code).unwrap_or_else(|| {
            unknown += 1;
            VerdictKind::NoFilter
        });
        let acc = out
            .entry((r.get(0).map_err(sql_err)?, source.as_str()))
            .or_insert_with(|| SummaryAcc {
                shape_json: r.get(2).unwrap_or_default(),
                source,
                executions: 0,
                misses: 0,
                total_millis: 0,
                max_millis: 0,
                worst: VerdictKind::NoFilter,
                buckets: BTreeMap::new(),
            });
        let start = bucket_start(r.get(3).map_err(sql_err)?, width);
        let (executions, misses) = (get(4)?, get(5)?);
        acc.executions = acc.executions.saturating_add(executions);
        acc.misses = acc.misses.saturating_add(misses);
        acc.total_millis = acc.total_millis.saturating_add(get(6)?);
        acc.max_millis = acc.max_millis.max(get(7)?);
        acc.worst = worst(acc.worst, verdict);
        let bucket = acc.buckets.entry(start).or_default();
        bucket.executions = bucket.executions.saturating_add(executions);
        bucket.misses = bucket.misses.saturating_add(misses);
        bucket.gap_secs = bucket.gap_secs.saturating_add(get(9)?);
    }
    if unknown > 0 {
        log::warn!("history database: {unknown} shape value(s) not understood by this build");
    }
    Ok(out)
}

fn read_gaps(
    conn: &Connection,
    connection: Uuid,
    db: &str,
    (from, to): (i64, i64),
    width: i64,
) -> Result<BTreeMap<i64, u64>> {
    let mut stmt = conn
        .prepare(
            "SELECT start_secs, gap_secs FROM shape_gap
             WHERE connection = ?1 AND db = ?2 AND start_secs >= ?3 AND start_secs <= ?4",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![connection.to_string(), db, from, to], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(sql_err)?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (start, gap) = row.map_err(sql_err)?;
        let slot: &mut u64 = out.entry(bucket_start(start, width)).or_default();
        *slot = slot.saturating_add(unsigned(gap));
    }
    Ok(out)
}
