//! One poll of the profiler: which databases are read, from where, and what happens to what was
//! read (record, then move the watermark, never the other way round).
//!
//! Limits worth knowing:
//! - A mark is kept per database. When the node a database is read from changes (a
//!   secondary-preferred connection), the next read counts as a first sight: no gap is judged,
//!   and entries of the new node older than the mark are not read.
//! - Dropping and recreating `system.profile` while the profiler stays on looks like a roll-off
//!   and may report a false `MissedEntries`.
//! - A database that flips between two nodes (A, B, A) is a first sight at every flip, and each
//!   node's rows are keyed by its host: entries of a node older than the mark are skipped, and a
//!   node's entries the mark has already passed are never read twice but may be missed.
//! - The mark is persisted with every recorded page (see `HistoryDb::record_page`), so a restart
//!   or reconnect continues after the newest entry already counted instead of counting the whole
//!   profile again. A database seen for the first time by a poller starts at the later of the
//!   retention floor and the stored mark (the newest over all nodes, since the node is unknown
//!   before the first read), and that start is NOT anchored: a restart cannot tell whether
//!   entries rolled off while the app was closed, so it never reports that as a loss. A database
//!   that disappears from the listing and returns is seeded from the stored mark the same way.
//! - Databases are polled in a rotating order: a pass that runs out of time resumes at the
//!   database it stopped at, so none starves.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::time::Instant;
use uuid::Uuid;

use super::super::profiler_status::ProfilerStatus;
use super::super::sampler::{
    Answer, SYSTEM_DBS, SamplerConfig, SamplerPause, Streak, TickError, blocking, bounded,
    pass_deadline, pause_reason, resume_point, rotated,
};
use super::fold_entries;
use crate::error::CoreError;
use crate::error_name::variant_of;
use crate::history::retention::RAW_RETENTION_SECS;
use crate::history::{HistoryDb, ProfileMark, RAW_BUCKET_SECS};
use crate::store::{DocumentStore, PROFILE_READ_LIMIT, ProfileRead};

/// Most pages one database is read for in one tick; a larger backlog goes on at the next tick.
const MAX_PAGES_PER_TICK: usize = 100;

const MILLIS_PER_SEC: i64 = 1_000;

/// Where reading a database continues.
#[derive(Clone)]
struct Mark {
    ts_millis: i64,
    /// Whether the position comes from entries actually read. Only then can entries missing after
    /// it mean a roll-off; the floor of a first sight is not a position the profile ever had.
    anchored: bool,
    /// The node the last read came from.
    host: Option<String>,
    /// The span already recorded as lost after this mark: `(mark, oldest)` of the last gap. A
    /// retry of a tick whose page failed to record must not count the same loss again.
    gap_done: Option<(i64, i64)>,
}

/// The per-database watermarks of one poller (a copy of the persisted marks, seeded at first
/// sight of a database).
#[derive(Default)]
pub(in super::super) struct Marks {
    dbs: HashMap<String, Mark>,
    /// Where the next pass starts in the listing (see `resume_point`).
    resume: usize,
}

impl Marks {
    /// Starts tracking `db` at `ts_millis`, unanchored; keeps an existing mark.
    fn seed(&mut self, db: &str, ts_millis: i64) {
        self.dbs.entry(db.to_owned()).or_insert(Mark {
            ts_millis,
            anchored: false,
            host: None,
            gap_done: None,
        });
    }

    /// The mark of `db`, created at `floor` the first time it is asked for.
    fn get_or_floor(&mut self, db: &str, floor: i64) -> Mark {
        self.dbs
            .entry(db.to_owned())
            .or_insert(Mark {
                ts_millis: floor,
                anchored: false,
                host: None,
                gap_done: None,
            })
            .clone()
    }

    /// Notes the node the latest read came from; another node than before is a first sight.
    fn observe_host(&mut self, db: &str, host: &str) {
        let Some(mark) = self.dbs.get_mut(db) else {
            return;
        };
        if mark.host.as_deref().is_some_and(|known| known != host) {
            mark.anchored = false;
            mark.gap_done = None;
        }
        mark.host = Some(host.to_owned());
    }

    /// Moves the mark to `newest` (never backwards) and anchors it. Returns whether it moved.
    fn advance(&mut self, db: &str, newest: Option<i64>) -> bool {
        let (Some(newest), Some(mark)) = (newest, self.dbs.get_mut(db)) else {
            return false;
        };
        let moved = newest > mark.ts_millis;
        if moved {
            mark.ts_millis = newest;
            mark.gap_done = None;
        }
        mark.anchored = true;
        moved
    }

    fn set_gap_done(&mut self, db: &str, span: (i64, i64)) {
        if let Some(mark) = self.dbs.get_mut(db) {
            mark.gap_done = Some(span);
        }
    }

    /// The profiler of `db` is off: nothing is written meanwhile, so what is missing when it
    /// is on again is no loss.
    fn unanchor(&mut self, db: &str) {
        if let Some(mark) = self.dbs.get_mut(db) {
            mark.anchored = false;
            mark.gap_done = None;
        }
    }

    /// Forgets the databases the server no longer lists.
    fn retain_listed(&mut self, listed: &[String]) {
        self.dbs.retain(|db, _| listed.contains(db));
    }

    pub(in super::super) fn tracks(&self, db: &str) -> bool {
        self.dbs.contains_key(db)
    }

    #[cfg(test)]
    pub(in super::super) fn resume(&self) -> usize {
        self.resume
    }
}

/// What one poll found.
pub(in super::super) struct Poll {
    /// Databases whose profile was read.
    reading: usize,
    /// The first database whose profile lost entries before they were read.
    missed: Option<String>,
}

impl Poll {
    pub(in super::super) fn status(self) -> ProfilerStatus {
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
    /// Skipped: the server cannot profile it.
    Skipped,
    /// The pass ran out of time before its profile was read; it counts as nothing and is
    /// revisited first by the next pass.
    Deferred,
    /// The user may not read it.
    Denied,
    /// A call failed in a way that is not a pause (a timeout, a server error).
    Failed(CoreError),
    Read {
        missed: bool,
    },
}

impl DbOutcome {
    /// How the database counts toward the run of timeouts that ends a pass.
    fn answer(&self) -> Answer {
        match self {
            Self::Failed(error) => Answer::of_error(error),
            Self::Skipped | Self::Denied | Self::Deferred => Answer::Failed,
            Self::Off | Self::Read { .. } => Answer::Read,
        }
    }
}

/// The long-lived parts of one poll.
struct Ctx<'a> {
    store: &'a dyn DocumentStore,
    history: &'a Arc<HistoryDb>,
    connection: Uuid,
    cfg: &'a SamplerConfig,
    floor: i64,
    deadline: Instant,
}

/// What the databases of one poll amounted to.
#[derive(Default)]
struct Tally {
    tried: usize,
    denied: usize,
    reading: usize,
    missed: Option<String>,
    first_failure: Option<CoreError>,
}

/// Polls every non-system, non-excluded database once.
///
/// A database that cannot be profiled (a mongos) or read (denied) is skipped. The poll fails with
/// `NotAuthorized` when every database it tried was denied, with the connection error when the
/// connection is lost, and with the first failure when nothing at all could be read although
/// something failed (so a dead node is never reported as idle). It stops early after
/// `MAX_CONSECUTIVE_TIMEOUTS` databases in a row timed out, and at the pass deadline. A history
/// failure fails the poll with the watermark of the failing page not moved, so the page is read
/// again.
pub(in super::super) async fn poll_once(
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
        deadline: Instant::now() + pass_deadline(cfg.interval),
    };
    let databases = bounded(cfg.command_timeout, store.list_databases())
        .await
        .map_err(TickError::Store)?;
    marks.retain_listed(&databases);
    let mut tally = Tally::default();
    let mut streak = Streak::default();
    let targets: Vec<String> = databases
        .into_iter()
        .filter(|db| !SYSTEM_DBS.contains(&db.as_str()) && !cfg.excluded_dbs.contains(db))
        .collect();
    let (total, start, mut completed) = (targets.len(), marks.resume, 0);
    for db in rotated(targets, start) {
        if Instant::now() >= ctx.deadline {
            log::warn!("profile poller: the pass ran past its deadline, the rest waits");
            break;
        }
        let outcome = poll_db(&ctx, &db, marks).await?;
        if matches!(outcome, DbOutcome::Deferred) {
            break;
        }
        completed += 1;
        streak.observe(outcome.answer());
        tally.take(&db, outcome);
        if streak.exhausted() {
            log::warn!("profile poller: calls in a row timed out, pass abandoned");
            break;
        }
    }
    // The next pass resumes where this one stopped, so no database starves behind slow ones.
    marks.resume = resume_point(start, completed, total);
    tally.finish()
}

impl Tally {
    fn take(&mut self, db: &str, outcome: DbOutcome) {
        self.tried += 1;
        match outcome {
            DbOutcome::Read { missed } => {
                self.reading += 1;
                if missed && self.missed.is_none() {
                    self.missed = Some(db.to_owned());
                }
            }
            DbOutcome::Denied => self.denied += 1,
            DbOutcome::Failed(error) => {
                self.first_failure.get_or_insert(error);
            }
            DbOutcome::Off | DbOutcome::Skipped | DbOutcome::Deferred => {}
        }
    }

    fn finish(self) -> Result<Poll, TickError> {
        if self.tried > 0 && self.denied == self.tried {
            return Err(TickError::Store(CoreError::Unauthorized));
        }
        match (self.reading, self.first_failure) {
            (0, Some(error)) => Err(TickError::Store(error)),
            _ => Ok(Poll {
                reading: self.reading,
                missed: self.missed,
            }),
        }
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
    if Instant::now() >= ctx.deadline {
        return Ok(DbOutcome::Deferred);
    }
    seed_from_history(ctx, db, marks).await?;
    let mut missed = false;
    for _ in 0..MAX_PAGES_PER_TICK {
        if Instant::now() >= ctx.deadline {
            break;
        }
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
        marks.observe_host(db, &read.host);
        let mark = marks.get_or_floor(db, ctx.floor);
        missed |= record_gap_if_lost(ctx, db, &mark, &read, marks).await?;
        record_page(ctx, db, &read).await?;
        let moved = marks.advance(db, read.newest_ts_millis);
        let full = read.entries.len() as u64 + read.skipped >= PROFILE_READ_LIMIT as u64;
        if !(full && moved) {
            break;
        }
    }
    Ok(DbOutcome::Read { missed })
}

/// First sight of `db` by this poller: continues after the stored mark, never earlier than the
/// retention floor.
async fn seed_from_history(ctx: &Ctx<'_>, db: &str, marks: &mut Marks) -> Result<(), TickError> {
    if marks.tracks(db) {
        return Ok(());
    }
    let (history, connection, db_name) = (Arc::clone(ctx.history), ctx.connection, db.to_owned());
    let stored = blocking(move || history.profile_mark(connection, &db_name))
        .await
        .map_err(TickError::history)?;
    marks.seed(db, stored.map_or(ctx.floor, |ts| ts.max(ctx.floor)));
    Ok(())
}

/// A failed profiler call: fatal for the connection, or only for this database.
fn skipped_or_fatal(error: CoreError) -> Result<DbOutcome, TickError> {
    match (&error, pause_reason(&error)) {
        (CoreError::Unsupported(_), _) => Ok(DbOutcome::Skipped),
        (_, Some(SamplerPause::NotAuthorized)) => Ok(DbOutcome::Denied),
        (_, Some(_)) => Err(TickError::Store(error)),
        (_, None) => {
            log::warn!("profile poller: a database failed: {}", variant_of(&error));
            Ok(DbOutcome::Failed(error))
        }
    }
}

/// Records that entries between the mark and the oldest one left in the profile were lost, when
/// the mark is a position the profile had, spread over the buckets the loss covers. Only the
/// part of the loss not recorded before is written, so the retry of a tick never counts it
/// twice. `true` while a loss applies to this read.
async fn record_gap_if_lost(
    ctx: &Ctx<'_>,
    db: &str,
    mark: &Mark,
    read: &ProfileRead,
    marks: &mut Marks,
) -> Result<bool, TickError> {
    let Some(oldest) = read
        .oldest_ts_millis
        .filter(|o| mark.anchored && *o > mark.ts_millis)
    else {
        return Ok(false);
    };
    let from = match mark.gap_done {
        Some((same_mark, recorded_to)) if same_mark == mark.ts_millis => recorded_to,
        _ => mark.ts_millis,
    };
    if oldest <= from {
        return Ok(true);
    }
    let span = u64::try_from(((oldest - from) / MILLIS_PER_SEC).max(1)).unwrap_or(1);
    let (history, connection) = (Arc::clone(ctx.history), ctx.connection);
    let (host, db_name) = (read.host.clone(), db.to_owned());
    let at = from.div_euclid(MILLIS_PER_SEC);
    blocking(move || history.record_gap_span(connection, &host, &db_name, at, span))
        .await
        .map_err(TickError::history)?;
    marks.set_gap_done(db, (mark.ts_millis, oldest));
    Ok(true)
}

/// Folds and records one page and moves the persisted mark past it, atomically; a page with
/// neither observations nor a newer position costs no history call.
async fn record_page(ctx: &Ctx<'_>, db: &str, read: &ProfileRead) -> Result<(), TickError> {
    let rows = fold_entries(
        ctx.connection,
        &read.host,
        db,
        &read.entries,
        RAW_BUCKET_SECS,
    );
    let mark = read.newest_ts_millis.map(|ts_millis| ProfileMark {
        connection: ctx.connection,
        host: read.host.clone(),
        db: db.to_owned(),
        ts_millis,
    });
    if rows.is_empty() && mark.is_none() {
        return Ok(());
    }
    let history = Arc::clone(ctx.history);
    blocking(move || history.record_page(&rows, mark.as_ref()))
        .await
        .map_err(TickError::history)
}
