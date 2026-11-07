//! Background poller of `system.profile`: once per interval it reads the new profile entries of
//! every database whose profiler is already on, folds them into literal-free query shapes and
//! appends those to the [`HistoryDb`].
//!
//! The poller only reads. It never enables the profiler (that is the person's explicit
//! `Command::SetProfilingLevel`) and never stores a literal value: only the shape of an entry
//! leaves [`fold_entries`]. Like the index sampler it reports on change only, pauses with a
//! growing wait on errors that mean "stop for now", and ends when its status channel closes.

mod pass;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, Interval};
use uuid::Uuid;

use super::profiler_status::ProfilerStatus;
use super::sampler::{
    BUSY_PASSES_BEFORE_REPORT, MIN_INTERVAL, SamplerConfig, SamplerPause, TickError, new_ticker,
    report, wait_for_tick,
};
use crate::clock;
use crate::error_name::variant_of;
use crate::history::HistoryDb;
use crate::store::DocumentStore;
pub(super) use pass::{Marks, Poll, poll_once};

pub use super::profiler_fold::fold_entries;

/// What the loop remembers between ticks.
struct LoopState {
    interval: Duration,
    ticker: Interval,
    /// Consecutive pause-class failures; zero while healthy.
    failures: u32,
    /// Consecutive ticks the history file was busy for.
    busy_ticks: u32,
    /// `Idle` from the start: a poller with nothing to read has nothing to say.
    reported: Option<ProfilerStatus>,
    marks: Marks,
}

impl LoopState {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            ticker: new_ticker(interval, true),
            failures: 0,
            busy_ticks: 0,
            reported: Some(ProfilerStatus::Idle),
            marks: Marks::default(),
        }
    }

    /// Picks up a changed interval from the next tick on.
    fn follow_interval(&mut self, interval: Duration) {
        if interval != self.interval {
            self.interval = interval;
            self.ticker = new_ticker(interval, false);
        }
    }

    /// The status a finished tick stands for, or `None` when it leaves the last one standing
    /// (a history file that is busy for a moment).
    fn status_of(&mut self, result: Result<Poll, TickError>) -> Option<ProfilerStatus> {
        match result {
            Ok(poll) => {
                self.failures = 0;
                self.busy_ticks = 0;
                Some(poll.status())
            }
            Err(TickError::HistoryBusy(_)) => {
                log::warn!("profile poller: the history file is busy, tick retried later");
                self.failures = 0;
                self.busy_ticks = self.busy_ticks.saturating_add(1);
                (self.busy_ticks >= BUSY_PASSES_BEFORE_REPORT)
                    .then_some(ProfilerStatus::Paused(SamplerPause::HistoryBusy))
            }
            Err(error) => {
                self.busy_ticks = 0;
                self.other_failure(&error)
            }
        }
    }

    fn other_failure(&mut self, error: &TickError) -> Option<ProfilerStatus> {
        match error.pause() {
            Some(reason) => {
                log::warn!("profile poller paused: {}", variant_of(error.error()));
                self.failures = self.failures.saturating_add(1);
                Some(ProfilerStatus::Paused(reason))
            }
            None => {
                // The server answers, so this is no reason to pause; but nothing was learned, so
                // the last status must not stay up.
                log::warn!("profile polling failed: {}", variant_of(error.error()));
                self.failures = 0;
                Some(ProfilerStatus::Failing)
            }
        }
    }
}

/// The poller loop. `now` is the wall clock in Unix seconds; tests pass a controlled one.
pub(super) async fn run(
    store: Arc<dyn DocumentStore>,
    history: Arc<HistoryDb>,
    connection: Uuid,
    cfg: watch::Receiver<SamplerConfig>,
    status: UnboundedSender<ProfilerStatus>,
    now: impl Fn() -> i64 + Send + Sync + 'static,
) {
    let mut state = LoopState::new(cfg.borrow().interval.max(MIN_INTERVAL));
    loop {
        if !wait_for_tick(&mut state.ticker, state.interval, state.failures, &status).await {
            return;
        }
        let current = cfg.borrow().clone().clamped();
        state.follow_interval(current.interval);
        let began = Instant::now();
        let result = poll_once(
            &*store,
            &history,
            connection,
            &current,
            now(),
            &mut state.marks,
        )
        .await;
        let next = state.status_of(result);
        settle_overrun(&mut state, began).await;
        if let Some(next) = next {
            if !report(&mut state.reported, &status, next) {
                return;
            }
        }
    }
}

/// After a healthy tick that took a whole interval or more, the tick it missed is already due
/// and would start the next one at once. Drops it so the next waits for the following tick.
async fn settle_overrun(state: &mut LoopState, began: Instant) {
    if state.failures == 0 && began.elapsed() >= state.interval {
        state.ticker.tick().await;
    }
}

/// Starts the poller of one connection. It polls at once, then every `cfg.interval`, and ends
/// when the task is aborted or `status` is closed.
pub fn spawn(
    store: Arc<dyn DocumentStore>,
    history: Arc<HistoryDb>,
    connection: Uuid,
    cfg: watch::Receiver<SamplerConfig>,
    status: UnboundedSender<ProfilerStatus>,
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
