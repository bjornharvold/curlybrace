//! A watermark that is ahead of the server's own clock.
//!
//! Entries are stamped by the server, so a mark more than a minute ahead of the server's clock
//! cannot come from entries of this server (a clock that was set back, a restored or copied
//! profile, a node with a skewed clock). Every read since it returns nothing, and a stored mark
//! only moves forward, so without a correction the poller would be blind until real time caught
//! up. The server's time comes from the `hello` every profile read already makes: no extra call.

use std::sync::Arc;

use uuid::Uuid;

use super::super::sampler::{TickError, blocking};
use super::pass::Marks;
use crate::history::HistoryDb;
use crate::store::ProfileRead;

/// How far ahead of the server's clock a mark may be before it counts as wrong. A little slack
/// covers a read that races the clock and node-to-node drift.
pub(super) const MARK_AHEAD_TOLERANCE_MS: i64 = 60_000;

/// The mark to read from: `mark_millis` itself, or the server's now when `mark_millis` is more
/// than [`MARK_AHEAD_TOLERANCE_MS`] ahead of it (the flag says it was moved). Without the
/// server's time the mark is kept.
pub(super) fn clamp_ahead(mark_millis: i64, server_now_millis: Option<i64>) -> (i64, bool) {
    match server_now_millis {
        Some(now) if mark_millis > now.saturating_add(MARK_AHEAD_TOLERANCE_MS) => (now, true),
        _ => (mark_millis, false),
    }
}

/// Moves the mark of `db` back to the server's time when `read` shows it is ahead, in memory
/// (unanchored: the span it skips is not a loss) and in the history (a stored mark only follows
/// pages forward, so it is lowered here). `true` when it moved: the read made with the old mark
/// is not used, the caller reads again.
pub(super) async fn move_back_if_ahead(
    history: &Arc<HistoryDb>,
    connection: Uuid,
    db: &str,
    mark_millis: i64,
    read: &ProfileRead,
    marks: &mut Marks,
) -> Result<bool, TickError> {
    let (to, clamped) = clamp_ahead(mark_millis, read.server_now_millis);
    if !clamped {
        return Ok(false);
    }
    marks.move_back(db, to);
    let (history, db_name) = (Arc::clone(history), db.to_owned());
    blocking(move || history.lower_profile_mark(connection, &db_name, to))
        .await
        .map_err(TickError::history)?;
    log::warn!("profile poller: a watermark ahead of the server clock was moved back");
    Ok(true)
}
