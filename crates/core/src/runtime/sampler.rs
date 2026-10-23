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
use crate::history::{HistoryDb, SAMPLE_INTERVAL_SECS, Sample, SeriesKey};
use crate::store::{CollectionKind, DocumentStore, IndexUsage, Namespace};

/// Databases the server owns; never sampled.
const SYSTEM_DBS: [&str; 3] = ["admin", "local", "config"];

/// Longest wait between two attempts while the sampler is paused.
const MAX_BACKOFF: Duration = Duration::from_secs(600);

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
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(SAMPLE_INTERVAL_SECS as u64),
            excluded_dbs: BTreeSet::new(),
            max_concurrency: 4,
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
/// Errors: a failure on one collection never hides the others, which are still recorded. If any
/// failure asks for a pause (see [`pause_reason`], or a history write failure) the first such
/// error is returned after the pass; any other per-collection error is logged and skipped,
/// because a collection can vanish between being listed and being read. A history failure comes
/// back as a `Database` error; use [`spawn`] to get it classified as `HistoryUnavailable`.
pub async fn sample_once(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    cfg: &SamplerConfig,
    now_secs: i64,
) -> Result<usize> {
    run_pass(store, history, connection, cfg, now_secs)
        .await
        .map_err(|e| e.error().clone())
}

async fn run_pass(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    cfg: &SamplerConfig,
    now_secs: i64,
) -> std::result::Result<usize, TickError> {
    let mut first_pause: Option<TickError> = None;
    let targets = discover(store, cfg, &mut first_pause)
        .await
        .map_err(TickError::Store)?;
    let width = cfg.max_concurrency.max(1);
    let mut results = stream::iter(targets)
        .map(|ns| async move {
            let outcome = sample_collection(store, history, connection, &ns, now_secs).await;
            (ns, outcome)
        })
        .buffer_unordered(width);
    let mut recorded = 0;
    while let Some((ns, outcome)) = results.next().await {
        match outcome {
            Ok(count) => recorded += count,
            Err(error) => note(
                error,
                &format!("{}.{}", ns.db, ns.collection),
                &mut first_pause,
            ),
        }
    }
    first_pause.map_or(Ok(recorded), Err)
}

/// Keeps the first pause-class `error` in `first_pause`; logs and drops every other one.
fn note(error: TickError, what: &str, first_pause: &mut Option<TickError>) {
    if error.pause().is_some() {
        log::warn!("index usage: {what}: {:?}", error.error());
        first_pause.get_or_insert(error);
    } else {
        log::warn!("index usage: skipping {what}: {:?}", error.error());
    }
}

/// The collections to sample: real collections of the non-system, non-excluded databases.
/// Views and time-series collections have no index usage worth keeping, and `system.*`
/// collections are the server's own; all are skipped without an error.
async fn discover(
    store: &dyn DocumentStore,
    cfg: &SamplerConfig,
    first_pause: &mut Option<TickError>,
) -> Result<Vec<Namespace>> {
    let mut targets = Vec::new();
    for db in store.list_databases().await? {
        if SYSTEM_DBS.contains(&db.as_str()) || cfg.excluded_dbs.contains(&db) {
            continue;
        }
        match store.list_collection_kinds(&db).await {
            Ok(kinds) => targets.extend(
                kinds
                    .into_iter()
                    .filter(|(name, kind)| {
                        *kind == CollectionKind::Collection && !name.starts_with("system.")
                    })
                    .map(|(name, _)| Namespace::new(db.as_str(), name)),
            ),
            Err(error) => note(TickError::Store(error), &db, first_pause),
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
) -> std::result::Result<usize, TickError> {
    let rows = store.index_stats(ns).await.map_err(TickError::Store)?;
    let samples: Vec<(SeriesKey, Sample)> = rows
        .into_iter()
        .map(|row| series_sample(connection, ns, row, now_secs))
        .collect();
    let history = Arc::clone(history);
    blocking(move || {
        samples
            .iter()
            .try_for_each(|(key, sample)| history.record(key, *sample))
            .map(|()| samples.len())
    })
    .await
    .map_err(TickError::History)
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
}

async fn run(
    store: Arc<dyn DocumentStore>,
    history: Arc<HistoryDb>,
    connection: Uuid,
    cfg: watch::Receiver<SamplerConfig>,
    status: UnboundedSender<SamplerStatus>,
    now: fn() -> i64,
) {
    let first = cfg.borrow().interval;
    let mut state = LoopState {
        interval: first,
        ticker: new_ticker(first, true),
        failures: 0,
        reported: None,
        last_thin: None,
    };
    loop {
        if !wait_for_next_tick(&mut state, &status).await {
            return;
        }
        let current = cfg.borrow().clone();
        if current.interval != state.interval {
            state.interval = current.interval;
            state.ticker = new_ticker(current.interval, false);
        }
        let began = Instant::now();
        let result = tick(&*store, &history, connection, &current, now(), &mut state).await;
        let next = match result {
            Ok(_) => {
                state.failures = 0;
                Some(SamplerStatus::Running)
            }
            Err(error) => match error.pause() {
                Some(reason) => {
                    log::warn!("index usage sampler paused: {:?}", error.error());
                    state.failures = state.failures.saturating_add(1);
                    Some(SamplerStatus::Paused(reason))
                }
                None => {
                    // The server answers and lets us in, so this is not a reason to pause.
                    log::warn!("index usage sampling failed: {:?}", error.error());
                    state.failures = 0;
                    None
                }
            },
        };
        if state.failures == 0 && began.elapsed() >= state.interval {
            // The pass overran: the tick it missed is already due and would start the next
            // pass at once. Drop it so the next pass waits for the following aligned tick.
            state.ticker.tick().await;
        }
        if let Some(next) = next.filter(|n| state.reported != Some(*n)) {
            if status.send(next).is_err() {
                return;
            }
            state.reported = Some(next);
        }
    }
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

/// One pass plus the hourly thinning, which only runs after a good pass.
async fn tick(
    store: &dyn DocumentStore,
    history: &Arc<HistoryDb>,
    connection: Uuid,
    cfg: &SamplerConfig,
    now_secs: i64,
    state: &mut LoopState,
) -> std::result::Result<usize, TickError> {
    let recorded = run_pass(store, history, connection, cfg, now_secs).await?;
    if state.last_thin.is_none_or(|at| at.elapsed() >= THIN_EVERY) {
        let history = Arc::clone(history);
        blocking(move || history.thin(now_secs))
            .await
            .map_err(TickError::History)?;
        state.last_thin = Some(Instant::now());
    }
    Ok(recorded)
}
