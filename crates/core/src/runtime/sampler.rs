//! Background sampler of `$indexStats`: once per interval it reads the usage counters of every
//! user collection of a connection and appends them to the [`HistoryDb`].
//!
//! [`sample_once`] is one pass; [`spawn`] runs passes forever. The task never fails silently:
//! an error that means "stop for now" (not authorized, connection lost, history unusable) is
//! reported as a [`SamplerStatus::Paused`] and retried with a growing wait; any other error is
//! logged and the next tick tries again. Every [`HistoryDb`] call is a short blocking call and
//! runs on the blocking pool.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use uuid::Uuid;

use crate::clock;
use crate::error::{CoreError, Result};
use crate::history::{HistoryDb, SAMPLE_INTERVAL_SECS, Sample, SeriesKey, max_continuous_secs};
use crate::store::{CollectionKind, DocumentStore, IndexUsage, Namespace};

/// Databases the server owns; never sampled.
const SYSTEM_DBS: [&str; 3] = ["admin", "local", "config"];

/// Longest wait between two attempts while the sampler is paused.
const MAX_BACKOFF: Duration = Duration::from_secs(600);

/// How many store calls in a row may time out before the pass gives up and reports `Failing`: a
/// dead connection that never errors would otherwise cost one timeout per collection, hours on a
/// large deployment, while the status still reads `Running`.
pub(super) const MAX_CONSECUTIVE_TIMEOUTS: usize = 3;

/// The text of the `Database` error a timed-out call becomes.
const TIMEOUT_TEXT: &str = "timeout";

/// How far ahead of the wall clock at record time a sample may be stamped. More means the clock
/// was stepped back since the pass began (a sample stamped by the old time would sit in the
/// future and block every later one), so the sample is skipped.
pub(super) const MAX_CLOCK_AHEAD_SECS: i64 = 300;

/// The wall clock in Unix seconds. Tests pass a controlled one.
pub(super) type Clock = dyn Fn() -> i64 + Send + Sync;

/// Shortest allowed time between two passes.
pub const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// How often the history is thinned.
const THIN_EVERY: Duration = Duration::from_secs(3_600);

/// Tunables of a sampler; the task re-reads them at the start of every tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SamplerConfig {
    /// Time between two passes.
    pub interval: Duration,
    /// Databases the person opted out of, on top of the system databases.
    pub excluded_dbs: BTreeSet<String>,
    /// Most `index_stats` calls in flight at once.
    pub max_concurrency: usize,
    /// Longest one store call of a pass may take. The runtime sets it to its own
    /// `command_timeout` when it starts.
    pub command_timeout: Duration,
}

impl SamplerConfig {
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
}

impl TickError {
    pub(super) fn pause(&self) -> Option<SamplerPause> {
        match self {
            Self::Store(error) => pause_reason(error),
            Self::History(_) => Some(SamplerPause::HistoryUnavailable),
        }
    }

    fn error(&self) -> &CoreError {
        match self {
            Self::Store(error) | Self::History(error) => error,
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
    run_pass(store, history, connection, cfg, clock)
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
    fn abandoned(self) -> Self {
        Self {
            learned: false,
            ..self
        }
    }
}

impl Health {
    fn note(&mut self, error: TickError, what: String) {
        self.failed.push((what, error.error().clone()));
        match error {
            TickError::History(_) => {
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
) -> std::result::Result<Pass, TickError> {
    let now_secs = clock();
    let mut health = Health::default();
    let targets = discover(store, cfg, &mut health)
        .await
        .map_err(TickError::Store)?;
    let width = cfg.max_concurrency.max(1);
    let limit = cfg.command_timeout;
    let mut results = stream::iter(targets)
        .map(|ns| async move {
            let outcome =
                sample_collection(store, history, connection, &ns, now_secs, clock, limit).await;
            (ns, outcome)
        })
        .buffer_unordered(width);
    let mut recorded = 0;
    let mut timeouts = Streak::default();
    while let Some((ns, outcome)) = results.next().await {
        timeouts.observe(matches!(&outcome, Err(error) if timed_out(error.error())));
        match outcome {
            Ok(count) => {
                recorded += count;
                health.answered += 1;
                health.read += 1;
            }
            Err(error) => health.note(error, format!("{}.{}", ns.db, ns.collection)),
        }
        if timeouts.exhausted() {
            log::warn!(
                "index usage: {MAX_CONSECUTIVE_TIMEOUTS} calls in a row timed out, pass abandoned"
            );
            // Dropping the stream cancels the calls still in flight.
            return health.finish(recorded).map(Pass::abandoned);
        }
    }
    health.finish(recorded)
}

/// Consecutive timed-out calls of a pass.
#[derive(Default)]
struct Streak(usize);

impl Streak {
    fn observe(&mut self, timed_out: bool) {
        self.0 = if timed_out { self.0 + 1 } else { 0 };
    }

    fn exhausted(&self) -> bool {
        self.0 >= MAX_CONSECUTIVE_TIMEOUTS
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
) -> Result<Vec<Namespace>> {
    let mut targets = Vec::new();
    let mut timeouts = Streak::default();
    for db in bounded(cfg.command_timeout, store.list_databases()).await? {
        if SYSTEM_DBS.contains(&db.as_str()) || cfg.excluded_dbs.contains(&db) {
            continue;
        }
        let listed = bounded(cfg.command_timeout, store.list_collection_kinds(&db)).await;
        timeouts.observe(matches!(&listed, Err(error) if timed_out(error)));
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
        .map_err(TickError::History)
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

fn new_ticker(period: Duration, immediate: bool) -> Interval {
    let start = if immediate {
        Instant::now()
    } else {
        Instant::now() + period
    };
    let mut ticker = tokio::time::interval_at(start, period.max(Duration::from_millis(1)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker
}

/// What the loop remembers between ticks.
struct LoopState {
    interval: Duration,
    ticker: Interval,
    /// Consecutive pause-class failures; zero while healthy.
    failures: u32,
    reported: Option<SamplerStatus>,
    last_thin: Option<Instant>,
    /// Names the last pass could not read, so a repeating set is logged once.
    last_failed: Vec<String>,
}

/// The sampler loop. `now` is the wall clock in Unix seconds; tests pass a controlled one.
pub(super) async fn run(
    store: Arc<dyn DocumentStore>,
    history: Arc<HistoryDb>,
    connection: Uuid,
    cfg: watch::Receiver<SamplerConfig>,
    status: UnboundedSender<SamplerStatus>,
    now: impl Fn() -> i64 + Send + Sync + 'static,
) {
    let mut state = LoopState::new(cfg.borrow().interval.max(MIN_INTERVAL));
    loop {
        if !wait_for_next_tick(&mut state, &status).await {
            return;
        }
        let current = cfg.borrow().clone().clamped();
        state.follow_interval(current.interval);
        let began = Instant::now();
        let result = tick(&*store, &history, connection, &current, &now, &mut state).await;
        let next = status_of(result, &mut state);
        settle_overrun(&mut state, began).await;
        if !report(&mut state, &status, next) {
            return;
        }
    }
}

impl LoopState {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            ticker: new_ticker(interval, true),
            failures: 0,
            reported: None,
            last_thin: None,
            last_failed: Vec::new(),
        }
    }

    /// Picks up a changed interval from the next tick on.
    fn follow_interval(&mut self, interval: Duration) {
        if interval != self.interval {
            self.interval = interval;
            self.ticker = new_ticker(interval, false);
        }
    }
}

/// The status a finished tick stands for, and the failure count it leaves behind: a pass that
/// read something resets it, a pause-class error raises it (and so backs the next wait off), any
/// other error resets it.
fn status_of(result: std::result::Result<Pass, TickError>, state: &mut LoopState) -> SamplerStatus {
    match result {
        Ok(pass) => {
            state.failures = 0;
            report_failed(&pass.failed, &mut state.last_failed);
            match pass.learned {
                true => SamplerStatus::Running,
                false => SamplerStatus::Failing,
            }
        }
        Err(error) => match error.pause() {
            Some(reason) => {
                log::warn!("index usage sampler paused: {:?}", error.error());
                state.failures = state.failures.saturating_add(1);
                SamplerStatus::Paused(reason)
            }
            None => {
                // The server answers and lets us in, so this is not a reason to pause; but the
                // pass learned nothing, so the last status must not stay up.
                log::warn!("index usage sampling failed: {:?}", error.error());
                state.failures = 0;
                SamplerStatus::Failing
            }
        },
    }
}

/// After a healthy pass that took a whole interval or more, the tick it missed is already due and
/// would start the next pass at once. Drops it so the next pass waits for the following aligned
/// tick.
async fn settle_overrun(state: &mut LoopState, began: Instant) {
    if state.failures == 0 && began.elapsed() >= state.interval {
        state.ticker.tick().await;
    }
}

/// Sends `next` when it differs from the last status sent. `false` when nobody listens any more.
fn report(
    state: &mut LoopState,
    status: &UnboundedSender<SamplerStatus>,
    next: SamplerStatus,
) -> bool {
    if state.reported == Some(next) {
        return true;
    }
    if status.send(next).is_err() {
        return false;
    }
    state.reported = Some(next);
    true
}

/// Waits for the next tick, or the backoff while paused. `false` when the status channel closed.
async fn wait_for_next_tick(
    state: &mut LoopState,
    status: &UnboundedSender<SamplerStatus>,
) -> bool {
    let wait = async {
        if state.failures == 0 {
            state.ticker.tick().await;
        } else {
            tokio::time::sleep(backoff(state.interval, state.failures)).await;
            // The ticks missed while backing off must not fire as a burst afterwards.
            state.ticker.reset();
        }
    };
    tokio::select! {
        () = wait => true,
        () = status.closed() => false,
    }
}

/// Logs the collections a pass could not read, when the set differs from the last pass's.
fn report_failed(failed: &[(String, CoreError)], last: &mut Vec<String>) {
    let names: Vec<String> = failed.iter().map(|(name, _)| name.clone()).collect();
    if names == *last {
        return;
    }
    if let Some((_, first_error)) = failed.first() {
        let shown: Vec<&str> = names.iter().take(3).map(String::as_str).collect();
        log::warn!(
            "index usage: {} collection(s) not sampled ({}{}): {:?}",
            names.len(),
            shown.join(", "),
            if names.len() > shown.len() {
                ", ..."
            } else {
                ""
            },
            first_error
        );
    }
    *last = names;
}

/// One pass, then the hourly thinning. Thinning has its own schedule: it runs after a failed
/// pass too (a paused sampler still thins), but not after a history failure, which it would
/// only repeat. A thinning failure is a history failure.
async fn tick(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    cfg: &SamplerConfig,
    clock: &Clock,
    state: &mut LoopState,
) -> std::result::Result<Pass, TickError> {
    let pass = run_pass(store, history, connection, cfg, clock).await;
    if matches!(pass, Err(TickError::History(_))) {
        return pass;
    }
    if state.last_thin.is_none_or(|at| at.elapsed() >= THIN_EVERY) {
        let history = Arc::clone(history);
        let max_continuous = max_continuous_secs(cfg.interval);
        // Thinning reads the clock afresh: a pass stamped before a step back must not decide what
        // is old.
        let now_secs = clock();
        blocking(move || history.thin(now_secs, max_continuous))
            .await
            .map_err(TickError::History)?;
        state.last_thin = Some(Instant::now());
    }
    pass
}
