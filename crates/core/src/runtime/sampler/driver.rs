//! The sampler's loop: when a pass runs, how it backs off and what it reports, plus the hourly
//! thinning. One pass itself is [`super::run_pass`].

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::time::{Instant, Interval, MissedTickBehavior};
use uuid::Uuid;

use super::{
    Clock, MIN_INTERVAL, Pass, SamplerConfig, SamplerStatus, TickError, backoff, blocking, run_pass,
};
use crate::error::CoreError;
use crate::history::{HistoryDb, is_history_busy, max_continuous_secs};
use crate::store::DocumentStore;

/// How often the history is thinned.
const THIN_EVERY: Duration = Duration::from_secs(3_600);

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
    /// Where the next pass starts in the target list (see [`Pass::next_start`]).
    cursor: usize,
}

/// The sampler loop. `now` is the wall clock in Unix seconds; tests pass a controlled one.
pub(in crate::runtime) async fn run(
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
            cursor: 0,
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
        Err(TickError::HistoryBusy(error)) => {
            // The file is busy for the moment: the whole pass is tried again at the next tick
            // and the sampler is no more paused than it was.
            log::warn!("index usage: the history file is busy, pass retried later: {error:?}");
            state.failures = 0;
            SamplerStatus::Running
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
    let pass = run_pass(store, history, connection, cfg, clock, state.cursor).await;
    if let Ok(done) = &pass {
        state.cursor = done.next_start;
    }
    if matches!(pass, Err(TickError::History(_) | TickError::HistoryBusy(_))) {
        return pass;
    }
    if state.last_thin.is_none_or(|at| at.elapsed() >= THIN_EVERY) {
        let history = Arc::clone(history);
        let max_continuous = max_continuous_secs(cfg.interval);
        // Thinning reads the clock afresh: a pass stamped before a step back must not decide what
        // is old.
        let now_secs = clock();
        let at = Instant::now();
        match blocking(move || history.thin_if_due(at, THIN_EVERY, now_secs, max_continuous)).await
        {
            Ok(_) => state.last_thin = Some(Instant::now()),
            // The pass is fine; thinning is tried again at the next tick.
            Err(error) if is_history_busy(&error) => {
                log::warn!("index usage: the history file is busy, thinning retried later");
            }
            Err(error) => return Err(TickError::History(error)),
        }
    }
    pass
}
