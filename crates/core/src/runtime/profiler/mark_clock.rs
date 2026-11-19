//! The profiler's watermark against the server's own clock.
//!
//! Entries are stamped by the server. A mark more than a minute ahead of the server's clock
//! cannot come from entries of a clock that keeps time (it was set back, a profile was restored
//! or copied): every read since it returns nothing until real time catches up. The server's time
//! comes from the `hello` every profile read already makes, so no call is added.
//!
//! Two rules keep this from counting anything twice:
//! - an entry stamped later than the server's now (plus the tolerance) is dropped from the read
//!   and never moves the mark past that point: it can only be one counted before the clock went
//!   back, or a stamp nobody can trust;
//! - a mark ahead of the server's clock is moved back in memory only, to the server's now but
//!   never below the retention floor, so a bogus time (0, a date long past) cannot reopen the
//!   profile's whole history. The stored mark is left alone (it only follows pages forward), so
//!   a restart simply meets the same rule again.
//!
//! What this costs: entries stamped after the clock went back and before the old mark are read
//! again only once the server's clock reaches them, and then once.

use super::pass::Marks;
use crate::store::ProfileRead;

/// How far ahead of the server's clock a stamp may be before it counts as wrong. A little slack
/// covers a read that races the clock and node-to-node drift.
pub(super) const MARK_AHEAD_TOLERANCE_MS: i64 = 60_000;

/// The mark to read from: `mark_millis` itself, or the server's now (never below `floor_millis`)
/// when `mark_millis` is more than [`MARK_AHEAD_TOLERANCE_MS`] ahead of it; the flag says it was
/// moved. Without the server's time, or when there is nowhere lower to go, the mark is kept.
pub(super) fn clamp_ahead(
    mark_millis: i64,
    server_now_millis: Option<i64>,
    floor_millis: i64,
) -> (i64, bool) {
    match server_now_millis {
        Some(now) if mark_millis > now.saturating_add(MARK_AHEAD_TOLERANCE_MS) => {
            let target = now.max(floor_millis);
            if target < mark_millis {
                (target, true)
            } else {
                (mark_millis, false)
            }
        }
        _ => (mark_millis, false),
    }
}

/// `read` without the entries stamped later than the server's now plus the tolerance. The newest
/// position never passes such a stamp, and an oldest entry that late says nothing about what
/// rolled off. A read without the server's time is returned as it is.
pub(super) fn without_future(read: ProfileRead) -> ProfileRead {
    let Some(cap) = read
        .server_now_millis
        .map(|now| now.saturating_add(MARK_AHEAD_TOLERANCE_MS))
    else {
        return read;
    };
    let entries: Vec<_> = read
        .entries
        .into_iter()
        .filter(|entry| entry.ts_millis <= cap)
        .collect();
    // The raw newest stamp covers documents that could not be parsed too (counted in `skipped`).
    // When it is past the cap the position stops at the newest entry kept (none: it does not
    // move), so neither a dropped entry nor an unreadable document stamped ahead of the clock
    // carries the mark past it, and a real entry stamped a moment later is never skipped.
    let newest_ts_millis = match read.newest_ts_millis {
        Some(newest) if newest > cap => entries.iter().map(|entry| entry.ts_millis).max(),
        other => other,
    };
    ProfileRead {
        entries,
        oldest_ts_millis: read.oldest_ts_millis.filter(|oldest| *oldest <= cap),
        newest_ts_millis,
        ..read
    }
}

/// Moves the mark of `db` back (unanchored: the span it skips is no loss) when `read` shows it
/// is ahead of the server's clock. `true` when it moved: the read made with the old mark holds
/// nothing, the caller reads again.
pub(super) fn move_back_if_ahead(
    floor_millis: i64,
    db: &str,
    mark_millis: i64,
    read: &ProfileRead,
    marks: &mut Marks,
) -> bool {
    let (to, clamped) = clamp_ahead(mark_millis, read.server_now_millis, floor_millis);
    if clamped {
        marks.move_back(db, to);
        log::warn!("profile poller: a watermark ahead of the server clock was moved back");
    }
    clamped
}
