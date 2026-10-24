//! Retention: rolling old samples into coarser rows, expiring them, and the size cap.
//!
//! Rolling uses `derive` + `bucketize`, so a rolled row holds exactly a `Bucket`'s fields: the
//! sum of deltas (gap uses and the uses counted since a restart included), the restart count and
//! the seconds of no data. Restarts and gaps therefore stay visible after thinning. Rows rolled
//! before restart uses were counted lack them.
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
//!
//! ## The deletion rules
//!
//! History is never deleted because of a wall-clock comparison that can be wrong; the clock may
//! be behind (a dead RTC before NTP) or ahead.
//!
//! - Samples and daily rows leave by age only: older than `now - retention`. A sample stamped
//!   ahead of the clock is never expired for that, however far; reads ignore it.
//! - A raw row is deleted only when the roll folded it: it is older than the baseline and not
//!   past what was rolled in the same transaction.
//! - A stored `rolled_through` ahead of the clock (it ran ahead and was corrected) is not
//!   trusted: the seam falls back to the oldest raw sample, so rolling resumes at once.
//! - Only the size cap may delete by stamp: it drops series of future-only rows first.

use super::db::sql_err;
use super::db_schema::{TIER_DAILY, TIER_HOURLY};
use super::retention::{
    DAILY_RETENTION_SECS, DAY_SECS, HOUR_SECS, HOURLY_RETENTION_SECS, RAW_RETENTION_SECS,
};
use super::seam;
use super::series::{MAX_FUTURE_SKEW_SECS, bucketize_within, derive};
use super::usage::{clip_gap, trusted_seam};
use crate::error::Result;
use rusqlite::{Connection, Transaction, params};

/// Rows deleted per step while enforcing the size cap.
const CAP_BATCH_ROWS: i64 = 500;

pub(super) fn thin(
    conn: &mut Connection,
    now_secs: i64,
    max_bytes: u64,
    max_continuous_secs: i64,
) -> Result<()> {
    let tx = conn.transaction().map_err(sql_err)?;
    roll_raw_into_hourly(&tx, now_secs, max_continuous_secs)?;
    roll_hourly_into_daily(&tx, now_secs)?;
    expire(&tx, now_secs)?;
    remove_empty_series(&tx)?;
    tx.commit().map_err(sql_err)?;
    release_free_pages(conn)?;
    enforce_cap(conn, max_bytes, now_secs)?;
    remove_empty_series(conn)
}

fn floor_to(secs: i64, width: i64) -> i64 {
    secs.div_euclid(width) * width
}

fn roll_raw_into_hourly(tx: &Transaction, now_secs: i64, max_continuous_secs: i64) -> Result<()> {
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
        roll_series(tx, series_id, now_secs, cutoff, max_continuous_secs)?;
    }
    Ok(())
}

fn roll_series(
    tx: &Transaction,
    series_id: i64,
    now_secs: i64,
    cutoff: i64,
    max_continuous_secs: i64,
) -> Result<()> {
    // Only samples `derive` accepts count: a backwards-clock sample is never the baseline, so
    // it cannot make newer data look old enough to delete.
    let Some(seam) = seam::load(tx, series_id, now_secs, cutoff)? else {
        return Ok(());
    };
    let baseline_id = seam.baseline_id;
    // Up to and including the accepted sample after the baseline, so a gap straddling the
    // cutoff is seen whole. When there is none, the silence is still open: the roll only goes as
    // far as the baseline, and the rest of it is rolled once a sample ends it.
    let window = seam.window;
    let rolled_to = if seam.has_next {
        cutoff
    } else {
        seam.baseline_at
    };
    let stored_through: i64 = tx
        .query_row(
            "SELECT rolled_through FROM series WHERE id = ?1",
            [series_id],
            |r| r.get(0),
        )
        .map_err(sql_err)?;
    // A seam ahead of the clock (the clock ran ahead and was corrected) must not suppress rolling.
    let rolled_through = trusted_seam(stored_through, now_secs, Some(seam.oldest_at));
    // The same clip the reads apply: seconds of a gap before `rolled_through` are already in
    // the rows, so a gap that continues past an earlier roll adds only its new part.
    let points: Vec<_> = derive(&window, max_continuous_secs)
        .into_iter()
        .map(|p| clip_gap(p, rolled_through))
        .collect();
    // Only buckets after `rolled_through` and before the cutoff are rolled, so only those are built.
    for bucket in bucketize_within(&points, HOUR_SECS, rolled_through, cutoff - 1) {
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
        "UPDATE series SET rolled_through = ?2 WHERE id = ?1",
        params![series_id, rolled_through.max(rolled_to)],
    )
    .map_err(sql_err)?;
    // Only rows the roll folded: older than the baseline and not past what was rolled. Never by
    // id alone: a row stamped ahead of the clock may have a lower id than the baseline (the clock
    // was behind when the baseline was written) and is real data the reads skip.
    tx.execute(
        "DELETE FROM samples WHERE series_id = ?1 AND id < ?2 AND at_secs <= ?3",
        params![series_id, baseline_id, rolled_to],
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

/// Daily rows, and any raw sample, older than the longest retention are deleted: plain age
/// retention, measured as "older than now - retention". Nothing is ever deleted for being
/// stamped ahead of the clock, however far: the clock may be the wrong one (a dead RTC before
/// NTP reads 1970, a restored VM is months behind), and every such row would then be real data.
/// Rows stamped ahead are ignored by reads and leave only by the size cap or, once the clock
/// reaches them, by age.
fn expire(tx: &Transaction, now_secs: i64) -> Result<()> {
    let cutoff = now_secs - DAILY_RETENTION_SECS;
    tx.execute(
        "DELETE FROM rolled WHERE tier = ?1 AND start_secs < ?2",
        params![TIER_DAILY, cutoff],
    )
    .map_err(sql_err)?;
    tx.execute("DELETE FROM samples WHERE at_secs < ?1", params![cutoff])
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

/// Deletes rows until the file fits: first the raw rows of series that hold no sample the reads
/// can see (all stamped ahead of the clock, whether or not they also have rolled rows), then the
/// oldest daily rows, then hourly, then raw.
///
/// The cap is the one place a wall-clock comparison deletes samples: with a clock *behind*, the
/// real rows of a dead series look like future ones and go first. That is acceptable, because
/// the cap is already deleting data; below the cap nothing is ever deleted for its stamp.
fn enforce_cap(conn: &Connection, max_bytes: u64, now_secs: i64) -> Result<()> {
    let believable_until = now_secs.saturating_add(MAX_FUTURE_SKEW_SECS);
    let steps = [
        (
            "DELETE FROM samples WHERE id IN (
                 SELECT id FROM samples
                 WHERE series_id NOT IN (SELECT series_id FROM samples WHERE at_secs <= ?2)
                 ORDER BY id LIMIT ?1)",
            true,
        ),
        (
            "DELETE FROM rolled WHERE (series_id, tier, start_secs) IN (
                 SELECT series_id, tier, start_secs FROM rolled WHERE tier = 2
                 ORDER BY start_secs LIMIT ?1)",
            false,
        ),
        (
            "DELETE FROM rolled WHERE (series_id, tier, start_secs) IN (
                 SELECT series_id, tier, start_secs FROM rolled WHERE tier = 1
                 ORDER BY start_secs LIMIT ?1)",
            false,
        ),
        (
            "DELETE FROM samples WHERE id IN (SELECT id FROM samples ORDER BY id LIMIT ?1)
                ",
            false,
        ),
    ];
    for (delete, takes_until) in steps {
        loop {
            if used_bytes(conn)? <= max_bytes {
                return Ok(());
            }
            let deleted = match takes_until {
                true => conn.execute(delete, params![CAP_BATCH_ROWS, believable_until]),
                false => conn.execute(delete, [CAP_BATCH_ROWS]),
            }
            .map_err(sql_err)?;
            release_free_pages(conn)?;
            if deleted == 0 {
                break;
            }
        }
    }
    Ok(())
}
