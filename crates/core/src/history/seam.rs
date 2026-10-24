//! The raw samples a roll needs: everything up to the first accepted sample after the cutoff.
//!
//! A roll looks at the accepted samples (see `accepted_indices`) from the oldest through the
//! baseline (the last one before the cutoff) and the one after it. What comes later cannot change
//! the roll, and which sample is accepted depends only on the ones before it, so the rows are read
//! in order and reading stops there instead of loading the whole raw window of the series.

use super::db::sql_err;
use super::series::{MAX_FUTURE_SKEW_SECS, Sample};
use crate::error::Result;
use rusqlite::{Connection, params};

/// The part of a series a roll derives from.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SeamWindow {
    /// Row id of the baseline, the last accepted sample before the cutoff.
    pub baseline_id: i64,
    pub baseline_at: i64,
    /// The accepted samples from the oldest through the baseline, plus the next one if any.
    pub window: Vec<Sample>,
    /// Whether an accepted sample follows the baseline (the silence after it is closed).
    pub has_next: bool,
    /// Time of the oldest accepted sample.
    pub oldest_at: i64,
}

/// The window of one series, or `None` when no accepted sample lies before `cutoff`. Rows
/// stamped more than [`MAX_FUTURE_SKEW_SECS`] after `now_secs` are left out, as in every read.
pub(super) fn load(
    conn: &Connection,
    series_id: i64,
    now_secs: i64,
    cutoff: i64,
) -> Result<Option<SeamWindow>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, at_secs, ops, since_ms FROM samples
             WHERE series_id = ?1 AND at_secs <= ?2 ORDER BY id",
        )
        .map_err(sql_err)?;
    let believable_until = now_secs.saturating_add(MAX_FUTURE_SKEW_SECS);
    let rows = stmt
        .query_map(params![series_id, believable_until], |r| {
            Ok((
                r.get(0)?,
                Sample {
                    at_secs: r.get(1)?,
                    ops: r.get(2)?,
                    since_ms: r.get(3)?,
                },
            ))
        })
        .map_err(sql_err)?
        .map(|row| row.map_err(sql_err));
    seam_window(rows, cutoff)
}

/// [`load`] over rows already in insertion order; stops reading at the first accepted sample
/// at or after `cutoff`.
pub(super) fn seam_window(
    rows: impl Iterator<Item = Result<(i64, Sample)>>,
    cutoff: i64,
) -> Result<Option<SeamWindow>> {
    let mut accepted: Vec<(i64, Sample)> = Vec::new();
    let mut baseline: Option<usize> = None;
    for row in rows {
        let (id, sample) = row?;
        // Accepted only when strictly after the last accepted one, like `accepted_indices`.
        if accepted
            .last()
            .is_some_and(|(_, last)| sample.at_secs <= last.at_secs)
        {
            continue;
        }
        accepted.push((id, sample));
        if sample.at_secs >= cutoff {
            break;
        }
        baseline = Some(accepted.len() - 1);
    }
    let Some(pos) = baseline else {
        return Ok(None);
    };
    let (baseline_id, baseline_sample) = accepted[pos];
    Ok(Some(SeamWindow {
        baseline_id,
        baseline_at: baseline_sample.at_secs,
        has_next: accepted.len() > pos + 1,
        oldest_at: accepted[0].1.at_secs,
        window: accepted.into_iter().map(|(_, sample)| sample).collect(),
    }))
}
