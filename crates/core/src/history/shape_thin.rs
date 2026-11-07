//! Retention for shape history: rolling buckets into coarser tiers, expiring them, and the
//! shape side of the size cap (see [`super::thin`], which drives all of it).
//!
//! Shape buckets are additive, so rolling is a plain `GROUP BY` that sums the counters, takes the
//! maximum of `max_millis` and the worst verdict (severity order, folded in Rust from the
//! distinct codes of the group), then adds the result to the coarser tier's row. The cutoffs are
//! the same whole-hour and whole-day cutoffs as the usage rows, so tiers never overlap.
//! Rows leave by age only (`now - retention`); a row stamped ahead of the clock is never expired
//! for that, only by the size cap.

use super::db::sql_err;
use super::db_schema::{TIER_DAILY, TIER_HOURLY};
use super::retention::{
    DAILY_RETENTION_SECS, DAY_SECS, HOUR_SECS, HOURLY_RETENTION_SECS, RAW_RETENTION_SECS,
};
use super::shape_schema::{COUNT_CEILING, TIER_RAW};
use super::shapes::{BucketValues, add_bucket, add_gap};
use crate::error::Result;
use crate::explain::VerdictKind;
use rusqlite::{Connection, params};

/// One step of the size cap, in the order `thin::enforce_cap` runs them.
#[derive(Debug, Clone, Copy)]
pub(super) enum CapStep {
    /// Raw buckets of shapes that hold no raw bucket the reads can see.
    FutureOnlyRaw,
    OldestDaily,
    OldestHourly,
    OldestRaw,
}

fn floor_to(secs: i64, width: i64) -> i64 {
    secs.div_euclid(width) * width
}

/// The most severe of comma-separated verdict codes; an unknown code ranks neutral and is kept
/// only when nothing better is in the group.
fn worst_code(codes: &str) -> String {
    let severity = |code: &str| VerdictKind::from_code(code).map_or(0, VerdictKind::severity);
    codes
        .split(',')
        .fold(None::<&str>, |best, code| match best {
            Some(b) if severity(code) <= severity(b) => Some(b),
            _ => Some(code),
        })
        .unwrap_or(VerdictKind::NoFilter.code())
        .to_owned()
}

/// Rolls, rolls again, then expires. Runs inside the thinning transaction.
pub(super) fn thin_shapes(conn: &Connection, now_secs: i64) -> Result<()> {
    let hourly_cutoff = floor_to(now_secs - RAW_RETENTION_SECS, HOUR_SECS);
    roll_buckets(conn, (TIER_RAW, TIER_HOURLY), HOUR_SECS, hourly_cutoff)?;
    roll_gaps(conn, (TIER_RAW, TIER_HOURLY), HOUR_SECS, hourly_cutoff)?;
    let daily_cutoff = floor_to(now_secs - HOURLY_RETENTION_SECS, DAY_SECS);
    roll_buckets(conn, (TIER_HOURLY, TIER_DAILY), DAY_SECS, daily_cutoff)?;
    roll_gaps(conn, (TIER_HOURLY, TIER_DAILY), DAY_SECS, daily_cutoff)?;
    let expiry = now_secs - DAILY_RETENTION_SECS;
    for table in ["shape_bucket", "shape_gap"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE tier = ?1 AND start_secs < ?2"),
            params![TIER_DAILY, expiry],
        )
        .map_err(sql_err)?;
    }
    Ok(())
}

fn roll_buckets(conn: &Connection, (from, to): (i64, i64), width: i64, cutoff: i64) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT shape_id, start_secs - (((start_secs % ?1) + ?1) % ?1) AS rolled_start,
                    MIN(SUM(executions), ?4), MIN(SUM(misses), ?4), MIN(SUM(total_millis), ?4),
                    MAX(max_millis), GROUP_CONCAT(DISTINCT worst), MIN(SUM(gap_secs), ?4)
             FROM shape_bucket WHERE tier = ?2 AND start_secs < ?3
             GROUP BY shape_id, rolled_start",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![width, from, cutoff, COUNT_CEILING], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                [
                    r.get::<_, i64>(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(7)?,
                ],
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(sql_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    for (shape_id, start, [executions, misses, total_millis, max_millis, gap_secs], codes) in rows {
        let worst = worst_code(&codes);
        let values = BucketValues {
            executions,
            misses,
            total_millis,
            max_millis,
            worst: &worst,
            gap_secs,
        };
        add_bucket(conn, shape_id, to, start, &values)?;
    }
    conn.execute(
        "DELETE FROM shape_bucket WHERE tier = ?1 AND start_secs < ?2",
        params![from, cutoff],
    )
    .map(drop)
    .map_err(sql_err)
}

fn roll_gaps(conn: &Connection, (from, to): (i64, i64), width: i64, cutoff: i64) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT connection, host, db, start_secs - (((start_secs % ?1) + ?1) % ?1) AS s,
                    MIN(SUM(gap_secs), ?4)
             FROM shape_gap WHERE tier = ?2 AND start_secs < ?3
             GROUP BY connection, host, db, s",
        )
        .map_err(sql_err)?;
    let rows = stmt
        .query_map(params![width, from, cutoff, COUNT_CEILING], |r| {
            Ok((
                [r.get::<_, String>(0)?, r.get(1)?, r.get(2)?],
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(sql_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    for ([connection, host, db], start, gap) in rows {
        add_gap(conn, (&connection, &host, &db), to, start, gap)?;
    }
    conn.execute(
        "DELETE FROM shape_gap WHERE tier = ?1 AND start_secs < ?2",
        params![from, cutoff],
    )
    .map(drop)
    .map_err(sql_err)
}

/// Forgets shapes that hold no bucket.
pub(super) fn remove_empty_shapes(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM shape WHERE id NOT IN (SELECT shape_id FROM shape_bucket)",
        [],
    )
    .map(drop)
    .map_err(sql_err)
}

/// Deletes up to `batch` of the oldest shape buckets of `step`; returns how many went.
/// `believable_until` is the latest stamp the clock can explain (for `FutureOnlyRaw`).
pub(super) fn cap_step(
    conn: &Connection,
    step: CapStep,
    batch: i64,
    believable_until: i64,
) -> Result<usize> {
    let (tier, future_only) = match step {
        CapStep::FutureOnlyRaw => (TIER_RAW, true),
        CapStep::OldestDaily => (TIER_DAILY, false),
        CapStep::OldestHourly => (TIER_HOURLY, false),
        CapStep::OldestRaw => (TIER_RAW, false),
    };
    conn.execute(
        "DELETE FROM shape_bucket WHERE (shape_id, tier, start_secs) IN (
             SELECT shape_id, tier, start_secs FROM shape_bucket
             WHERE tier = ?1
               AND (?4 = 0 OR shape_id NOT IN (
                        SELECT shape_id FROM shape_bucket WHERE tier = ?1 AND start_secs <= ?3))
             ORDER BY start_secs LIMIT ?2)",
        params![tier, batch, believable_until, i64::from(future_only)],
    )
    .map_err(sql_err)
}

/// The stamp of the oldest bucket `step` would delete, if any (to decide whose turn is first).
pub(super) fn oldest(conn: &Connection, step: CapStep) -> Result<Option<i64>> {
    let tier = match step {
        CapStep::FutureOnlyRaw => return Ok(None),
        CapStep::OldestDaily => TIER_DAILY,
        CapStep::OldestHourly => TIER_HOURLY,
        CapStep::OldestRaw => TIER_RAW,
    };
    conn.query_row(
        "SELECT MIN(start_secs) FROM shape_bucket WHERE tier = ?1",
        [tier],
        |r| r.get(0),
    )
    .map_err(sql_err)
}
