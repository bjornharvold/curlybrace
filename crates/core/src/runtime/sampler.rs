//! Background sampler of `$indexStats`: once per interval it reads the usage counters of every
//! user collection of a connection and appends them to the [`HistoryDb`].
//!
//! [`sample_once`] is one pass; [`spawn`] runs passes forever. The task never fails silently:
//! an error that means "stop for now" (not authorized, connection lost, history unusable) is
//! reported as a [`SamplerStatus::Paused`] and retried with a growing wait; any other error is
//! logged and the next tick tries again. Every [`HistoryDb`] call is a short blocking call and
//! runs on the blocking pool.

mod driver;
pub(super) use driver::run;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use uuid::Uuid;

use crate::clock;
use crate::error::{CoreError, Result};
use crate::history::{HistoryDb, SAMPLE_INTERVAL_SECS, Sample, SeriesKey, is_history_busy};
use crate::store::{CollectionKind, DocumentStore, IndexUsage, Namespace};

/// Databases the server owns; never sampled.
const SYSTEM_DBS: [&str; 3] = ["admin", "local", "config"];

/// Longest wait between two attempts while the sampler is paused.
const MAX_BACKOFF: Duration = Duration::from_secs(600);

/// How many store calls in a row may time out (or fail like a dead connection) before the pass
/// gives up and reports `Failing`: a dead connection that never errors would otherwise cost one
/// timeout per collection, hours on a large deployment, while the status still reads `Running`.
/// A call that reads data restarts the count. The pass scales this by its concurrency: one
/// network blip fails every call in flight at once, and that burst is one event, not several.
pub(super) const MAX_CONSECUTIVE_TIMEOUTS: usize = 3;

/// A pass may last this many intervals at most, whatever its calls did, so a connection that
/// answers every other call cannot hold a pass (and the status `Running`) for hours.
const PASS_DEADLINE_INTERVALS: u32 = 10;

/// The shortest pass deadline, for short intervals.
const MIN_PASS_DEADLINE: Duration = Duration::from_secs(600);

/// The wall time after which a pass, discovery included, is cut off: ten intervals, at least ten
/// minutes. A call already in flight at that moment is cancelled. A pass that was reading
/// healthily (almost no timeouts) keeps `Running` and the next pass resumes where it stopped;
/// otherwise it is reported `Failing`.
pub(super) fn pass_deadline(interval: Duration) -> Duration {
    interval
        .saturating_mul(PASS_DEADLINE_INTERVALS)
        .max(MIN_PASS_DEADLINE)
}

/// The text of the `Database` error a timed-out call becomes.
const TIMEOUT_TEXT: &str = "timeout";

/// How far ahead of the wall clock at record time a sample may be stamped. More means the clock
/// was stepped back since the pass began (a sample stamped by the old time would sit in the
/// future and block every later one), so the sample is skipped.
pub(super) const MAX_CLOCK_AHEAD_SECS: i64 = crate::history::MAX_FUTURE_SKEW_SECS;

/// The wall clock in Unix seconds. Tests pass a controlled one.
pub(super) type Clock = dyn Fn() -> i64 + Send + Sync;

/// Shortest allowed time between two passes.
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Tunables of a sampler; the task re-reads them at the start of every tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SamplerConfig {
    /// Time between two passes.
    pub interval: Duration,
    /// Databases the person opted out of, on top of the system databases.
    pub excluded_dbs: BTreeSet<String>,
    /// Most `index_stats` calls in flight at once.
    pub max_concurrency: usize,
    /// Longest one store call of a pass may take. Not public: the runtime derives it from
    /// `RuntimeConfig::command_timeout`, the single source of truth, when it starts.
    pub(crate) command_timeout: Duration,
}

impl SamplerConfig {
    /// The same tunables with another time between two passes.
    pub fn with_interval(self, interval: Duration) -> Self {
        Self { interval, ..self }
    }

    /// The same tunables with the interval raised to [`MIN_INTERVAL`]: a zero interval would make
    /// a paused sampler retry in a tight loop.
    pub fn clamped(self) -> Self {
        Self {
            interval: self.interval.max(MIN_INTERVAL),
            ..self
        }
    }
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(SAMPLE_INTERVAL_SECS as u64),
            excluded_dbs: BTreeSet::new(),
            max_concurrency: 4,
            command_timeout: super::COMMAND_TIMEOUT,
        }
    }
}

/// Why the sampler is not collecting. The UI maps each to an i18n key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplerPause {
    /// The server refused `$indexStats` (or the login) for this user.
    NotAuthorized,
    /// The server cannot be reached.
    ConnectionLost,
    /// The history database could not be written.
    HistoryUnavailable,
}

/// What the sampler reports on its status channel, on change only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplerStatus {
    Running,
    /// The sampler runs but its last pass learned nothing because every read failed with an
    /// error that is not worth a pause (see the log for the cause). It retries every interval.
    Failing,
    Paused(SamplerPause),
}

/// Why a pass failed: the server side or the history side. The history side always pauses the
/// sampler, whatever the text, because `HistoryDb` reports every failure as `Database`.
pub(super) enum TickError {
    Store(CoreError),
    History(CoreError),
    /// The history file was busy or locked past the wait. Unlike `History` this passes by
    /// itself, so it does not pause the sampler: the pass is tried again at the next tick.
    HistoryBusy(CoreError),
}

impl TickError {
    /// A history failure, told apart by whether the file was only busy.
    pub(super) fn history(error: CoreError) -> Self {
        match is_history_busy(&error) {
            true => Self::HistoryBusy(error),
            false => Self::History(error),
        }
    }
}

impl TickError {
    pub(super) fn pause(&self) -> Option<SamplerPause> {
        match self {
            Self::Store(error) => pause_reason(error),
            Self::History(_) => Some(SamplerPause::HistoryUnavailable),
            Self::HistoryBusy(_) => None,
        }
    }

    fn error(&self) -> &CoreError {
        match self {
            Self::Store(error) | Self::History(error) | Self::HistoryBusy(error) => error,
        }
    }
}

/// The pause a server error asks for, or `None` when the error is only worth a log line (for
/// example a `Database` error for one collection that was dropped since it was listed).
pub fn pause_reason(error: &CoreError) -> Option<SamplerPause> {
    match error {
        CoreError::Unauthorized | CoreError::AuthFailed => Some(SamplerPause::NotAuthorized),
        CoreError::Unreachable
        | CoreError::UnreachableSsh
        | CoreError::ConnectionInterrupted(_) => Some(SamplerPause::ConnectionLost),
        _ => None,
    }
}

/// The wait after `failures` consecutive pause-class passes: the interval doubled per failure,
/// at most ten minutes (or one interval when that is longer).
pub(super) fn backoff(interval: Duration, failures: u32) -> Duration {
    let factor = 2u32.saturating_pow(failures.min(16));
    interval
        .saturating_mul(factor)
        .min(MAX_BACKOFF.max(interval))
}

/// One pass over every user collection of the connection. Returns how many series got a sample.
///
/// Errors: a failure on one collection never hides the others, which are still recorded. The
/// pass only fails when it learned nothing: discovery failed (`list_databases`), or every
/// listing and every collection failed with an error that asks for a pause (see
/// [`pause_reason`]), for instance a user denied on the whole server. If any database or
/// collection answered, the denied or failed ones are only reported in [`Pass::failed`]: a user
/// who can read `shop` but not `audit` keeps a history of `shop`. A history write failure always
/// fails the pass; it comes back as a `Database` error, and [`spawn`] classifies it as
/// `HistoryUnavailable`.
// The loop uses `run_pass` directly; this wrapper is how the tests observe one pass.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn sample_once(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    cfg: &SamplerConfig,
    clock: &Clock,
) -> Result<usize> {
    run_pass(store, history, connection, cfg, clock, 0)
        .await
        .map(|pass| pass.recorded)
        .map_err(|e| e.error().clone())
}

/// What a pass that did not fail as a whole found.
struct Pass {
    #[cfg_attr(not(test), allow(dead_code))]
    recorded: usize,
    /// The collections (or databases) that could not be read, by name, with their error.
    failed: Vec<(String, CoreError)>,
    /// Whether anything was read at all: false when every database or collection failed.
    learned: bool,
    /// Where the next pass starts in the target list: where this one stopped when it was cut
    /// short, so a pass too short for the whole deployment still reaches every collection in
    /// turn; zero after a complete pass.
    next_start: usize,
}

/// What a pass learned about the server and the history, to decide whether it failed.
#[derive(Default)]
struct Health {
    /// Collections read without an error, plus databases listed that had none to read.
    answered: usize,
    /// Collections whose usage was actually read and recorded; a database with nothing to read
    /// adds none, so it cannot make a pass of failures look like a pass that learned something.
    read: usize,
    first_pause: Option<TickError>,
    history: Option<TickError>,
    failed: Vec<(String, CoreError)>,
}

impl Pass {
    /// The pass cut short: whatever it read stays recorded, but it did not learn enough to call
    /// the connection healthy.
    fn abandoned(self, next_start: usize) -> Self {
        Self {
            learned: false,
            next_start,
            ..self
        }
    }

    /// The pass hit its deadline while reading healthily and slowly: it keeps its status but the
    /// next pass resumes where this one stopped.
    fn resumed_at(self, next_start: usize) -> Self {
        Self { next_start, ..self }
    }
}

impl Health {
    fn note(&mut self, error: TickError, what: String) {
        self.failed.push((what, error.error().clone()));
        match error {
            TickError::History(_) | TickError::HistoryBusy(_) => {
                self.history.get_or_insert(error);
            }
            TickError::Store(_) if error.pause().is_some() => {
                self.first_pause.get_or_insert(error);
            }
            TickError::Store(_) => {}
        }
    }

    fn finish(mut self, recorded: usize) -> std::result::Result<Pass, TickError> {
        if let Some(history) = self.history {
            return Err(history);
        }
        match self.first_pause {
            Some(pause) if self.answered == 0 => Err(pause),
            _ => {
                self.failed.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Pass {
                    next_start: 0,
                    recorded,
                    learned: self.read > 0 || self.failed.is_empty(),
                    failed: self.failed,
                })
            }
        }
    }
}

async fn run_pass(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    cfg: &SamplerConfig,
    clock: &Clock,
    start: usize,
) -> std::result::Result<Pass, TickError> {
    let now_secs = clock();
    let deadline = Instant::now() + pass_deadline(cfg.interval);
    let mut health = Health::default();
    let targets = discover(store, cfg, &mut health, deadline)
        .await
        .map_err(TickError::Store)?;
    let width = cfg.max_concurrency.max(1);
    let limit = cfg.command_timeout;
    let mut progress = Progress::new(health, start, targets.len(), width);
    let mut results = stream::iter(rotated(targets, start))
        .map(|ns| async move {
            let outcome =
                sample_collection(store, history, connection, &ns, now_secs, clock, limit).await;
            (ns, outcome)
        })
        .buffer_unordered(width);
    loop {
        let Ok(next) = tokio::time::timeout_at(deadline, results.next()).await else {
            return progress.past_deadline();
        };
        let Some((ns, outcome)) = next else { break };
        // Dropping the stream on a return cancels the calls still in flight.
        if progress.observe(&ns, outcome) {
            return progress.abandoned();
        }
    }
    progress.health.finish(progress.recorded)
}

/// What a running pass has seen so far.
struct Progress {
    health: Health,
    start: usize,
    total: usize,
    completed: usize,
    timed_out: usize,
    recorded: usize,
    timeouts: Streak,
}

impl Progress {
    fn new(health: Health, start: usize, total: usize, width: usize) -> Self {
        Self {
            health,
            start,
            total,
            completed: 0,
            timed_out: 0,
            recorded: 0,
            timeouts: Streak::for_concurrency(width),
        }
    }

    /// Takes one collection's outcome in. `true` when the pass must stop: the history cannot be
    /// written, or too many calls in a row timed out.
    fn observe(&mut self, ns: &Namespace, outcome: std::result::Result<usize, TickError>) -> bool {
        self.completed += 1;
        let answer = match &outcome {
            Ok(_) => Answer::Read,
            Err(error) => Answer::of_error(error.error()),
        };
        self.timed_out += usize::from(matches!(answer, Answer::TimedOut));
        self.timeouts.observe(answer);
        match outcome {
            Ok(count) => {
                self.recorded += count;
                self.health.answered += 1;
                self.health.read += 1;
            }
            Err(error) => self
                .health
                .note(error, format!("{}.{}", ns.db, ns.collection)),
        }
        if self.health.history.is_some() {
            // Every further read would be thrown away: the pass fails with this error.
            log::warn!("index usage: the history cannot be written, pass ended");
            return true;
        }
        let exhausted = self.timeouts.exhausted();
        if exhausted {
            log::warn!(
                "index usage: {} calls in a row timed out, pass abandoned",
                self.timeouts.limit
            );
        }
        exhausted
    }

    fn resume(&self) -> usize {
        resume_point(self.start, self.completed, self.total)
    }

    fn abandoned(self) -> std::result::Result<Pass, TickError> {
        let resume = self.resume();
        self.health
            .finish(self.recorded)
            .map(|pass| pass.abandoned(resume))
    }

    /// The pass hit its deadline: healthy-but-slow keeps its status and resumes next time,
    /// anything else is abandoned.
    fn past_deadline(self) -> std::result::Result<Pass, TickError> {
        let slow_but_healthy =
            self.health.read > 0 && self.timed_out * SLOW_TOLERANCE <= self.health.read;
        if !slow_but_healthy {
            log::warn!("index usage: the pass ran past its deadline, abandoned");
            return self.abandoned();
        }
        log::warn!(
            "index usage: the pass ran past its deadline after {} of {} collections; the next \
             pass resumes there",
            self.completed,
            self.total
        );
        let resume = self.resume();
        self.health
            .finish(self.recorded)
            .map(|pass| pass.resumed_at(resume))
    }
}

/// A deadline-cut pass counts as slow but healthy while at most one call in this many timed out.
const SLOW_TOLERANCE: usize = 10;

/// The targets starting at `start` (wrapping), so consecutive cut-short passes cover the whole list.
fn rotated(mut targets: Vec<Namespace>, start: usize) -> Vec<Namespace> {
    if !targets.is_empty() {
        let by = start % targets.len();
        targets.rotate_left(by);
    }
    targets
}

/// Where the pass that began at `start` and finished `completed` targets leaves the next one.
fn resume_point(start: usize, completed: usize, total: usize) -> usize {
    match total {
        0 => 0,
        total => (start % total + completed) % total,
    }
}

/// How one call of a pass ended.
#[derive(Clone, Copy)]
pub(super) enum Answer {
    /// The call returned data.
    Read,
    /// The call ran into the command timeout.
    TimedOut,
    /// The call failed some other way.
    Failed,
}

impl Answer {
    /// How a failed call counts: a timeout and a dead-connection error both show a server that
    /// does not answer, anything else (a denied or dropped collection) does not.
    pub(super) fn of_error(error: &CoreError) -> Self {
        match error {
            CoreError::Unreachable
            | CoreError::TlsHandshake
            | CoreError::ConnectionInterrupted(_) => Self::TimedOut,
            error if timed_out(error) => Self::TimedOut,
            _ => Self::Failed,
        }
    }
}

/// Consecutive calls of a pass that timed out or hit a dead connection. A read restarts the
/// count, so a few slow collections never starve the ones after them; a run of them ends the pass
/// however many calls succeeded before it.
pub(super) struct Streak {
    timeouts: usize,
    limit: usize,
}

impl Default for Streak {
    fn default() -> Self {
        Self::for_concurrency(1)
    }
}

impl Streak {
    /// A streak that ends the pass after [`MAX_CONSECUTIVE_TIMEOUTS`] bursts of `width` calls.
    pub(super) fn for_concurrency(width: usize) -> Self {
        Self {
            timeouts: 0,
            limit: MAX_CONSECUTIVE_TIMEOUTS.saturating_mul(width.max(1)),
        }
    }

    pub(super) fn observe(&mut self, answer: Answer) {
        match answer {
            Answer::Read | Answer::Failed => self.timeouts = 0,
            Answer::TimedOut => self.timeouts += 1,
        }
    }

    pub(super) fn exhausted(&self) -> bool {
        self.timeouts >= self.limit
    }
}

fn timed_out(error: &CoreError) -> bool {
    matches!(error, CoreError::Database(text) if text == TIMEOUT_TEXT)
}

/// The collections to sample: real collections of the non-system, non-excluded databases.
/// Views and time-series collections have no index usage worth keeping, and `system.*`
/// collections are the server's own; all are skipped without an error.
async fn discover(
    store: &dyn DocumentStore,
    cfg: &SamplerConfig,
    health: &mut Health,
    deadline: Instant,
) -> Result<Vec<Namespace>> {
    let mut targets = Vec::new();
    let mut timeouts = Streak::default();
    for db in bounded(cfg.command_timeout, store.list_databases()).await? {
        if SYSTEM_DBS.contains(&db.as_str()) || cfg.excluded_dbs.contains(&db) {
            continue;
        }
        if Instant::now() >= deadline {
            return Err(CoreError::Database(TIMEOUT_TEXT.into()));
        }
        let listed = bounded(cfg.command_timeout, store.list_collection_kinds(&db)).await;
        timeouts.observe(match &listed {
            Ok(_) => Answer::Read,
            Err(error) => Answer::of_error(error),
        });
        if timeouts.exhausted() {
            return Err(CoreError::Database(TIMEOUT_TEXT.into()));
        }
        match listed {
            Ok(kinds) => {
                let before = targets.len();
                targets.extend(
                    kinds
                        .into_iter()
                        .filter(|(name, kind)| {
                            *kind == CollectionKind::Collection && !name.starts_with("system.")
                        })
                        .map(|(name, _)| Namespace::new(db.as_str(), name)),
                );
                // A listing only proves access when there is nothing left to read in it; a
                // database with collections proves it through reading them.
                if targets.len() == before {
                    health.answered += 1;
                }
            }
            Err(error) => health.note(TickError::Store(error), db),
        }
    }
    Ok(targets)
}

/// Reads the usage of one collection and records one sample per (node, index) series.
async fn sample_collection(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    ns: &Namespace,
    now_secs: i64,
    clock: &Clock,
    limit: Duration,
) -> std::result::Result<usize, TickError> {
    let rows = bounded(limit, store.index_stats(ns))
        .await
        .map_err(TickError::Store)?;
    let samples: Vec<(SeriesKey, Sample)> = rows
        .into_iter()
        .map(|row| series_sample(connection, ns, row, now_secs))
        .collect();
    let samples = plausible(samples, clock(), ns);
    let history = Arc::clone(history);
    // One transaction per collection, not per index: one commit instead of an fsync each.
    blocking(move || history.record_many(&samples).map(|()| samples.len()))
        .await
        .map_err(TickError::history)
}

/// The samples that are not stamped implausibly far ahead of `wall_now`, the wall clock at record
/// time. The sampler stamps a pass at its start; if the clock was stepped back since, the stamp
/// would put the sample in the future, and `derive` ignores every later sample until real time
/// catches up. Dropped samples are logged.
fn plausible(
    samples: Vec<(SeriesKey, Sample)>,
    wall_now: i64,
    ns: &Namespace,
) -> Vec<(SeriesKey, Sample)> {
    let limit = wall_now.saturating_add(MAX_CLOCK_AHEAD_SECS);
    let (kept, future): (Vec<_>, Vec<_>) = samples
        .into_iter()
        .partition(|(_, sample)| sample.at_secs <= limit);
    if !future.is_empty() {
        log::warn!(
            "index usage: {} sample(s) of {}.{} skipped, stamped {}s ahead of the clock",
            future.len(),
            ns.db,
            ns.collection,
            future[0].1.at_secs.saturating_sub(wall_now)
        );
    }
    kept
}

fn series_sample(
    connection: Uuid,
    ns: &Namespace,
    row: IndexUsage,
    now_secs: i64,
) -> (SeriesKey, Sample) {
    let key = SeriesKey {
        connection,
        host: row.host,
        db: ns.db.clone(),
        collection: ns.collection.clone(),
        index_name: row.name,
        // The definition the server returned, not one we guess: an index recreated with other
        // options starts a new series.
        definition_hash: row.spec.definition_hash(),
    };
    let sample = Sample {
        at_secs: now_secs,
        ops: row.ops,
        since_ms: row.since_ms,
    };
    (key, sample)
}

/// Bounds a store call by `limit`. The timeout is a `Database` error, which is not a pause
/// reason: the pass counts it as a failure and the next tick tries again.
async fn bounded<T>(limit: Duration, call: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(limit, call)
        .await
        .unwrap_or_else(|_| Err(CoreError::Database(TIMEOUT_TEXT.into())))
}

/// Runs a blocking `HistoryDb` call on the blocking pool. A panic there becomes an error.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|e| Err(CoreError::Database(format!("history task failed: {e}"))))
}

/// Starts the sampler of one connection. It samples at once, then every `cfg.interval`, and ends
/// when the task is aborted or `status` is closed. A tick that overruns the interval makes the
/// task skip the ticks it missed instead of running them back to back.
pub fn spawn(
    store: Arc<dyn DocumentStore>,
    history: Arc<HistoryDb>,
    connection: Uuid,
    cfg: watch::Receiver<SamplerConfig>,
    status: UnboundedSender<SamplerStatus>,
) -> JoinHandle<()> {
    tokio::spawn(run(
        store,
        history,
        connection,
        cfg,
        status,
        clock::now_secs,
    ))
}
