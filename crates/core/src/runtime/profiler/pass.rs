//! One poll of the profiler: which databases are read, from where, and what happens to what was
//! read (record, then move the watermark, never the other way round).

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use super::super::profiler_status::ProfilerStatus;
use super::super::sampler::SamplerPause;
use super::super::sampler::{
    SYSTEM_DBS, SamplerConfig, TickError, blocking, bounded, pause_reason,
};
use super::fold_entries;
use crate::error::CoreError;
use crate::history::retention::RAW_RETENTION_SECS;
use crate::history::{HistoryDb, RAW_BUCKET_SECS};
use crate::store::{DocumentStore, PROFILE_READ_LIMIT, ProfileRead};

/// Most pages one database is read for in one tick; a larger backlog goes on at the next tick.
const MAX_PAGES_PER_TICK: usize = 100;

const MILLIS_PER_SEC: i64 = 1_000;

/// Where reading a database continues.
#[derive(Clone, Copy)]
struct Mark {
    ts_millis: i64,
    /// Whether the position comes from entries actually read. Only then can entries missing after
    /// it mean a roll-off; the floor of a first sight is not a position the profile ever had.
    anchored: bool,
}

/// The per-database watermarks of one poller (in memory: a restart starts afresh from the
/// retention floor).
#[derive(Default)]
pub(super) struct Marks(HashMap<String, Mark>);

impl Marks {
    /// The mark of `db`, created at `floor` the first time it is asked for.
    fn get_or_floor(&mut self, db: &str, floor: i64) -> Mark {
        *self.0.entry(db.to_owned()).or_insert(Mark {
            ts_millis: floor,
            anchored: false,
        })
    }

    /// Moves the mark to `newest` (never backwards) and anchors it. Returns whether it moved.
    fn advance(&mut self, db: &str, newest: Option<i64>) -> bool {
        let Some(newest) = newest else { return false };
        let Some(mark) = self.0.get_mut(db) else {
            return false;
        };
        let moved = newest > mark.ts_millis;
        *mark = Mark {
            ts_millis: mark.ts_millis.max(newest),
            anchored: true,
        };
        moved
    }

    /// The profiler of `db` is off: nothing is written meanwhile, so what is missing when it
    /// is on again is no loss.
    fn unanchor(&mut self, db: &str) {
        if let Some(mark) = self.0.get_mut(db) {
            mark.anchored = false;
        }
    }
}

/// What one poll found.
pub(super) struct Poll {
    /// Databases whose profile was read.
    reading: usize,
    /// The first database whose profile lost entries before they were read.
    missed: Option<String>,
}

impl Poll {
    pub(super) fn status(self) -> ProfilerStatus {
        match (self.missed, self.reading) {
            (Some(db), _) => ProfilerStatus::MissedEntries { db },
            (None, 0) => ProfilerStatus::Idle,
            (None, databases) => ProfilerStatus::Reading { databases },
        }
    }
}

/// How one database ended.
enum DbOutcome {
    /// Profiler off; nothing was read.
    Off,
    /// Skipped: the server cannot profile it, or the call failed in a way not worth a pause.
    Skipped,
    /// The user may not read it.
    Denied,
    Read {
        missed: bool,
    },
}

/// The long-lived parts of one poll.
struct Ctx<'a> {
    store: &'a dyn DocumentStore,
    history: &'a Arc<HistoryDb>,
    connection: Uuid,
    cfg: &'a SamplerConfig,
    floor: i64,
}

/// Polls every non-system, non-excluded database once.
///
/// A database that cannot be profiled (a mongos) or read (denied) is skipped; the poll fails with
/// `NotAuthorized` only when every database it tried was denied, and with the connection error
/// when the connection is lost. A history failure fails the poll with the watermark of the
/// failing page not moved, so the page is read again.
pub(super) async fn poll_once(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    cfg: &SamplerConfig,
    now_secs: i64,
    marks: &mut Marks,
) -> Result<Poll, TickError> {
    let ctx = Ctx {
        store,
        history,
        connection,
        cfg,
        floor: now_secs.saturating_sub(RAW_RETENTION_SECS) * MILLIS_PER_SEC,
    };
    let databases = bounded(cfg.command_timeout, store.list_databases())
        .await
        .map_err(TickError::Store)?;
    let (mut tried, mut denied, mut poll) = (
        0usize,
        0usize,
        Poll {
            reading: 0,
            missed: None,
        },
    );
    for db in databases
        .iter()
        .filter(|db| !SYSTEM_DBS.contains(&db.as_str()) && !cfg.excluded_dbs.contains(*db))
    {
        tried += 1;
        match poll_db(&ctx, db, marks).await {
            Ok(DbOutcome::Read { missed }) => {
                poll.reading += 1;
                if missed && poll.missed.is_none() {
                    poll.missed = Some(db.clone());
                }
            }
            Ok(DbOutcome::Denied) => denied += 1,
            Ok(DbOutcome::Off | DbOutcome::Skipped) => {}
            Err(error) => return Err(error),
        }
    }
    match tried > 0 && denied == tried {
        true => Err(TickError::Store(CoreError::Unauthorized)),
        false => Ok(poll),
    }
}

async fn poll_db(ctx: &Ctx<'_>, db: &str, marks: &mut Marks) -> Result<DbOutcome, TickError> {
    let state = match bounded(ctx.cfg.command_timeout, ctx.store.profiling_level(db)).await {
        Ok(state) => state,
        Err(error) => return skipped_or_fatal(error),
    };
    if state.level <= 0 {
        marks.unanchor(db);
        return Ok(DbOutcome::Off);
    }
    let mut missed = false;
    for _ in 0..MAX_PAGES_PER_TICK {
        let mark = marks.get_or_floor(db, ctx.floor);
        let read = match bounded(
            ctx.cfg.command_timeout,
            ctx.store.read_profile(db, mark.ts_millis),
        )
        .await
        {
            Ok(read) => read,
            Err(error) => return skipped_or_fatal(error),
        };
        missed |= record_gap_if_lost(ctx, db, mark, &read).await?;
        record_page(ctx, db, &read).await?;
        let moved = marks.advance(db, read.newest_ts_millis);
        let full = read.entries.len() as u64 + read.skipped >= PROFILE_READ_LIMIT as u64;
        if !(full && moved) {
            break;
        }
    }
    Ok(DbOutcome::Read { missed })
}

/// A failed profiler call: fatal for the connection, or only for this database.
fn skipped_or_fatal(error: CoreError) -> Result<DbOutcome, TickError> {
    match (&error, pause_reason(&error)) {
        (CoreError::Unsupported(_), _) => Ok(DbOutcome::Skipped),
        (_, Some(SamplerPause::NotAuthorized)) => Ok(DbOutcome::Denied),
        (_, Some(_)) => Err(TickError::Store(error)),
        (_, None) => {
            log::warn!(
                "profile poller: a database was skipped: {}",
                variant_of(&error)
            );
            Ok(DbOutcome::Skipped)
        }
    }
}

/// The name of an error's variant, for logs that must not carry its payload.
pub(in super::super) fn variant_of(error: &CoreError) -> String {
    format!("{error:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

/// Records that entries between the mark and the oldest one left in the profile were lost, when
/// the mark is a position the profile had. `true` when a loss was recorded.
async fn record_gap_if_lost(
    ctx: &Ctx<'_>,
    db: &str,
    mark: Mark,
    read: &ProfileRead,
) -> Result<bool, TickError> {
    let Some(oldest) = read
        .oldest_ts_millis
        .filter(|o| mark.anchored && *o > mark.ts_millis)
    else {
        return Ok(false);
    };
    let gap = u64::try_from(((oldest - mark.ts_millis) / MILLIS_PER_SEC).max(1)).unwrap_or(1);
    let (history, connection) = (Arc::clone(ctx.history), ctx.connection);
    let (host, db) = (read.host.clone(), db.to_owned());
    let at = mark.ts_millis.div_euclid(MILLIS_PER_SEC);
    blocking(move || history.record_gap(connection, &host, &db, at, gap))
        .await
        .map_err(TickError::history)?;
    Ok(true)
}

/// Folds and records one page; a page without observations costs no history call.
async fn record_page(ctx: &Ctx<'_>, db: &str, read: &ProfileRead) -> Result<(), TickError> {
    let rows = fold_entries(
        ctx.connection,
        &read.host,
        db,
        &read.entries,
        RAW_BUCKET_SECS,
    );
    if rows.is_empty() {
        return Ok(());
    }
    let history = Arc::clone(ctx.history);
    blocking(move || history.record_shapes(&rows))
        .await
        .map_err(TickError::history)
}
