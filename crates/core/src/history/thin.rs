//! Retention: rolling old samples into coarser rows, expiring them, and the size cap.
//!
//! Rolling uses `derive` + `bucketize`, so a rolled row holds exactly a `Bucket`'s fields: the
//! sum of deltas (gap uses included), the restart count and the seconds of no data. Restarts
//! and gaps therefore stay visible after thinning.
//!
//! ## The seam between rolled and raw data
//!
//! Raw samples older than the cutoff (48 hours back, floored to a whole hour) are rolled into
//! hourly rows, **except the last of them, which stays as the baseline**. The next thinning,
//! and every chart read, derives from that baseline, so the pair of samples straddling the
//! cutoff still produces its delta (at the later sample's time, which is raw time) and is
//! neither lost nor counted twice. The cutoff is stored as the series' `rolled_through`:
//! hourly rows are only ever written for hours before it and starting at the previous value,
//! so no hour is rolled twice. To keep the seconds of a gap that straddles the cutoff, the
//! roll derives over the baseline plus the next sample and keeps only the buckets before the
//! cutoff; reads clip that gap's earlier part from the raw side (see [`super::usage`]).

use super::db::sql_err;
use super::db_schema::{TIER_DAILY, TIER_HOURLY};
use super::retention::{
    DAILY_RETENTION_SECS, DAY_SECS, HOUR_SECS, HOURLY_RETENTION_SECS, RAW_RETENTION_SECS,
};
use super::series::{accepted_indices, bucketize, derive};
use super::usage::{clip_gap, raw_samples};
use crate::error::Result;
use rusqlite::{Connection, Transaction, params};

/// Rows deleted per step while enforcing the size cap.
const CAP_BATCH_ROWS: i64 = 500;

pub(super) fn thin(conn: &mut Connection, now_secs: i64, max_bytes: u64) -> Result<()> {
    let tx = conn.transaction().map_err(sql_err)?;
    roll_raw_into_hourly(&tx, now_secs)?;
    roll_hourly_into_daily(&tx, now_secs)?;
    expire(&tx, now_secs)?;
    remove_empty_series(&tx)?;
    tx.commit().map_err(sql_err)?;
    release_free_pages(conn)?;
    enforce_cap(conn, max_bytes)?;
    remove_empty_series(conn)
}

fn floor_to(secs: i64, width: i64) -> i64 {
    secs.div_euclid(width) * width
}

fn roll_raw_into_hourly(tx: &Transaction, now_secs: i64) -> Result<()> {
    let cutoff = floor_to(now_secs - RAW_RETENTION_SECS, HOUR_SECS);
    let mut stmt = tx
        .prepare("SELECT DISTINCT series_id FROM samples WHERE at_secs < ?1")
        .map_err(sql_err)?;
    let ids = stmt
        .query_map([cutoff], |r| r.get::<_, i64>(0))
        .map_err(sql_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    for series_id in ids {
        roll_series(tx, series_id, cutoff)?;
    }
    Ok(())
}

fn roll_series(tx: &Transaction, series_id: i64, cutoff: i64) -> Result<()> {
    let samples = raw_samples(tx, series_id)?;
    // Only samples `derive` accepts count: a backwards-clock sample is never the baseline, so
    // it cannot make newer data look old enough to delete.
    let plain: Vec<_> = samples.iter().map(|(_, s)| *s).collect();
    let accepted = accepted_indices(&plain);
    // The baseline: the last accepted sample before the cutoff.
    let Some(pos) = accepted.iter().rposition(|&i| plain[i].at_secs < cutoff) else {
        return Ok(());
    };
    let baseline = accepted[pos];
    let baseline_id = samples[baseline].0;
    // Up to and including the accepted sample after it, so a gap straddling the cutoff is seen
    // whole. When there is none, the silence is still open: the roll only goes as far as the
    // baseline, and the rest of it is rolled once a sample ends it.
    let next = accepted.get(pos + 1);
    let window: Vec<_> = accepted[..=(pos + usize::from(next.is_some()))]
        .iter()
        .map(|&i| plain[i])
        .collect();
    let rolled_to = if next.is_some() {
        cutoff
    } else {
        plain[baseline].at_secs
    };
    let rolled_through: i64 = tx
        .query_row(
            "SELECT rolled_through FROM series WHERE id = ?1",
            [series_id],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    // The same clip the reads apply: seconds of a gap before `rolled_through` are already in
    // the rows, so a gap that continues past an earlier roll adds only its new part.
    let points: Vec<_> = derive(&window)
        .into_iter()
        .map(|p| clip_gap(p, rolled_through))
        .collect();
    for bucket in bucketize(&points, HOUR_SECS) {
        if bucket.start_secs + bucket.width_secs > rolled_through && bucket.start_secs < cutoff {
            add_row(
                tx,
                series_id,
                TIER_HOURLY,
                bucket.start_secs,
                (bucket.uses, i64::from(bucket.restarts), bucket.gap_secs),
            )?;
        }
    }
    tx.execute(
        "UPDATE series SET rolled_through = MAX(rolled_through, ?2) WHERE id = ?1",
        params![series_id, rolled_to],
    )
    .map_err(sql_err)?;
    tx.execute(
        "DELETE FROM samples WHERE series_id = ?1 AND id < ?2",
        params![series_id, baseline_id],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Adds to a rolled row, creating it when absent.
fn add_row(
    tx: &Transaction,
    series_id: i64,
    tier: i64,
    start_secs: i64,
    (uses, restarts, gap_secs): (i64, i64, i64),
) -> Result<()> {
    tx.execute(
        "INSERT INTO rolled (series_id, tier, start_secs, uses, restarts, gap_secs)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (series_id, tier, start_secs) DO UPDATE SET
             uses = uses + excluded.uses,
             restarts = restarts + excluded.restarts,
             gap_secs = gap_secs + excluded.gap_secs",
        params![series_id, tier, start_secs, uses, restarts, gap_secs],
    )
    .map(drop)
    .map_err(sql_err)
}

/// Hourly rows from before a whole-day cutoff are summed per day, so no day is ever split.
fn roll_hourly_into_daily(tx: &Transaction, now_secs: i64) -> Result<()> {
    let cutoff = floor_to(now_secs - HOURLY_RETENTION_SECS, DAY_SECS);
    tx.execute(
        "INSERT INTO rolled (series_id, tier, start_secs, uses, restarts, gap_secs)
         SELECT series_id, ?1, start_secs - (((start_secs % ?2) + ?2) % ?2),
                SUM(uses), SUM(restarts), SUM(gap_secs)
         FROM rolled WHERE tier = ?3 AND start_secs < ?4
         GROUP BY 1, 3
         ON CONFLICT (series_id, tier, start_secs) DO UPDATE SET
             uses = uses + excluded.uses,
             restarts = restarts + excluded.restarts,
             gap_secs = gap_secs + excluded.gap_secs",
        params![TIER_DAILY, DAY_SECS, TIER_HOURLY, cutoff],
    )
    .map_err(sql_err)?;
    tx.execute(
        "DELETE FROM rolled WHERE tier = ?1 AND start_secs < ?2",
        params![TIER_HOURLY, cutoff],
    )
    .map_err(sql_err)?;
    Ok(())
}

/// Daily rows, and any raw sample, older than the longest retention are deleted.
fn expire(tx: &Transaction, now_secs: i64) -> Result<()> {
    let cutoff = now_secs - DAILY_RETENTION_SECS;
    tx.execute(
        "DELETE FROM rolled WHERE tier = ?1 AND start_secs < ?2",
        params![TIER_DAILY, cutoff],
    )
    .map_err(sql_err)?;
    tx.execute("DELETE FROM samples WHERE at_secs < ?1", [cutoff])
        .map_err(sql_err)?;
    Ok(())
}

/// Forgets series that hold neither samples nor rolled rows.
fn remove_empty_series(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM series
         WHERE id NOT IN (SELECT series_id FROM samples)
           AND id NOT IN (SELECT series_id FROM rolled)",
        [],
    )
    .map(drop)
    .map_err(sql_err)
}

/// Bytes held by live pages. Free pages left by deletes do not count against the cap.
pub(super) fn used_bytes(conn: &Connection) -> Result<u64> {
    let pragma = |name: &str| -> Result<u64> {
        conn.pragma_query_value(None, name, |r| r.get::<_, i64>(0))
            .map(|v| u64::try_from(v).unwrap_or(0))
            .map_err(sql_err)
    };
    let live_pages = pragma("page_count")?.saturating_sub(pragma("freelist_count")?);
    Ok(live_pages * pragma("page_size")?)
}

/// Gives free pages back to the file (a no-op for a file created without incremental vacuum).
pub(super) fn release_free_pages(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA incremental_vacuum;")
        .map_err(sql_err)
}

/// Deletes the oldest rows, daily first, then hourly, then raw, until the file fits.
fn enforce_cap(conn: &Connection, max_bytes: u64) -> Result<()> {
    let steps = [
        "DELETE FROM rolled WHERE (series_id, tier, start_secs) IN (
             SELECT series_id, tier, start_secs FROM rolled WHERE tier = 2
             ORDER BY start_secs LIMIT ?1)",
        "DELETE FROM rolled WHERE (series_id, tier, start_secs) IN (
             SELECT series_id, tier, start_secs FROM rolled WHERE tier = 1
             ORDER BY start_secs LIMIT ?1)",
        "DELETE FROM samples WHERE id IN (SELECT id FROM samples ORDER BY id LIMIT ?1)",
    ];
    for delete in steps {
        loop {
            if used_bytes(conn)? <= max_bytes {
                return Ok(());
            }
            let deleted = conn.execute(delete, [CAP_BATCH_ROWS]).map_err(sql_err)?;
            release_free_pages(conn)?;
            if deleted == 0 {
                break;
            }
        }
    }
    Ok(())
}
