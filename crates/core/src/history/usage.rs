//! Reads a chart's buckets out of the store, merging raw samples with rolled rows.
//!
//! - Raw samples (read in insertion order) go through `derive` and `bucketize` at the requested
//!   width.
//! - Rolled rows (hourly, daily) become buckets of their own width. A row narrower than the
//!   request is summed into the requested bucket holding its start; a row at least as wide is
//!   returned as it is, so a 10 minute chart over hourly rows shows the hourly bucket (the
//!   bucket's `width_secs` says so). That only happens when a chart reaches past the 48 hour raw
//!   window, which the 24 hour range never does.
//! - A gap that began before the raw/rolled seam (`rolled_through`) was already counted, up to
//!   the seam, in the rolled rows; its seconds before the seam are clipped from the raw side so
//!   they are not counted twice. Its uses sit at its far end, in raw time, and are kept.
//! - Buckets are returned oldest first, limited to the window that ends at `now_secs`.

use super::db::{SeriesFacts, SeriesKey, sql_err};
use super::retention::ChartRange;
use super::series::{Bucket, Point, Sample, bucketize, derive};
use crate::error::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;

use super::db_schema::TIER_HOURLY;
use super::retention::{DAY_SECS, HOUR_SECS};

/// The row id and `rolled_through` of a series, when it exists.
pub(super) fn find_series(conn: &Connection, key: &SeriesKey) -> Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT id, rolled_through FROM series
         WHERE connection = ?1 AND host = ?2 AND db = ?3 AND collection = ?4
           AND index_name = ?5 AND definition_hash = ?6",
        params![
            key.connection.to_string(),
            key.host,
            key.db,
            key.collection,
            key.index_name,
            key.definition_hash
        ],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(sql_err)
}

/// Raw samples of a series in insertion order.
pub(super) fn raw_samples(conn: &Connection, series_id: i64) -> Result<Vec<(i64, Sample)>> {
    let mut stmt = conn
        .prepare("SELECT id, at_secs, ops, since_ms FROM samples WHERE series_id = ?1 ORDER BY id")
        .map_err(sql_err)?;
    let rows = stmt
        .query_map([series_id], |r| {
            Ok((
                r.get(0)?,
                Sample {
                    at_secs: r.get(1)?,
                    ops: r.get(2)?,
                    since_ms: r.get(3)?,
                },
            ))
        })
        .map_err(sql_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_err)
}

pub(super) fn usage(
    conn: &Connection,
    key: &SeriesKey,
    range: ChartRange,
    now_secs: i64,
) -> Result<Vec<Bucket>> {
    let Some((series_id, rolled_through)) = find_series(conn, key)? else {
        return Ok(Vec::new());
    };
    let width = range.bucket_width_secs();
    let window_start = now_secs - range.secs();

    let samples: Vec<Sample> = raw_samples(conn, series_id)?
        .into_iter()
        .map(|(_, sample)| sample)
        .collect();
    let points: Vec<Point> = derive(&samples)
        .into_iter()
        .map(|p| clip_gap(p, rolled_through))
        .collect();

    let mut merged: BTreeMap<(i64, i64), Bucket> = BTreeMap::new();
    for bucket in bucketize(&points, width) {
        add(&mut merged, bucket);
    }
    for row in rolled_rows(conn, series_id, window_start)? {
        add(&mut merged, widen(row, width));
    }
    Ok(merged
        .into_values()
        .filter(|b| b.start_secs + b.width_secs > window_start && b.start_secs <= now_secs)
        .collect())
}

/// The time of the newest use and of the oldest data, over everything stored (no range).
pub(super) fn facts(conn: &Connection, key: &SeriesKey) -> Result<SeriesFacts> {
    let Some((series_id, _)) = find_series(conn, key)? else {
        return Ok(SeriesFacts::default());
    };
    let samples: Vec<Sample> = raw_samples(conn, series_id)?
        .into_iter()
        .map(|(_, sample)| sample)
        .collect();
    let raw_use = derive(&samples)
        .into_iter()
        .filter_map(|p| match p {
            Point::Delta { at_secs, uses, .. } if uses > 0 => Some(at_secs),
            Point::Restart {
                at_secs,
                uses_since_restart,
            } if uses_since_restart > 0 => Some(at_secs),
            Point::Gap { to_secs, uses, .. } if uses > 0 => Some(to_secs),
            _ => None,
        })
        .max();
    let (rolled_use, rolled_first): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT MAX(CASE WHEN uses > 0
                        THEN start_secs + CASE tier WHEN ?2 THEN ?3 ELSE ?4 END END),
                    MIN(start_secs)
             FROM rolled WHERE series_id = ?1",
            params![series_id, TIER_HOURLY, HOUR_SECS, DAY_SECS],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql_err)?;
    let raw_first = samples.iter().map(|s| s.at_secs).min();
    Ok(SeriesFacts {
        last_used_at_secs: raw_use.or(rolled_use),
        tracked_since_secs: [rolled_first, raw_first].into_iter().flatten().min(),
    })
}

/// A gap's seconds before the seam are already in the rolled rows.
pub(super) fn clip_gap(point: Point, rolled_through: i64) -> Point {
    match point {
        Point::Gap {
            from_secs,
            to_secs,
            uses,
        } => Point::Gap {
            from_secs: from_secs.max(rolled_through).min(to_secs),
            to_secs,
            uses,
        },
        other => other,
    }
}

fn rolled_rows(conn: &Connection, series_id: i64, window_start: i64) -> Result<Vec<Bucket>> {
    let mut stmt = conn
        .prepare(
            "SELECT tier, start_secs, uses, restarts, gap_secs FROM rolled
             WHERE series_id = ?1 AND start_secs + CASE tier WHEN ?2 THEN ?3 ELSE ?4 END > ?5
             ORDER BY start_secs",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(
            params![series_id, TIER_HOURLY, HOUR_SECS, DAY_SECS, window_start],
            |r| {
                let tier: i64 = r.get(0)?;
                Ok(Bucket {
                    start_secs: r.get(1)?,
                    width_secs: if tier == TIER_HOURLY {
                        HOUR_SECS
                    } else {
                        DAY_SECS
                    },
                    uses: r.get(2)?,
                    restarts: r.get(3)?,
                    gap_secs: r.get(4)?,
                })
            },
        )
        .map_err(sql_err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_err)
}

/// Re-buckets a rolled row to `width` when it is narrower; keeps it as it is otherwise.
fn widen(row: Bucket, width: i64) -> Bucket {
    if row.width_secs >= width {
        return row;
    }
    Bucket {
        start_secs: row.start_secs.div_euclid(width) * width,
        width_secs: width,
        ..row
    }
}

fn add(merged: &mut BTreeMap<(i64, i64), Bucket>, bucket: Bucket) {
    merged
        .entry((bucket.start_secs, bucket.width_secs))
        .and_modify(|b| {
            b.uses = b.uses.saturating_add(bucket.uses);
            b.restarts += bucket.restarts;
            b.gap_secs += bucket.gap_secs;
        })
        .or_insert(bucket);
}
