//! The per-stage counts of the Aggregate tab: one bounded count per enabled stage, sent one after
//! another (never in parallel), and what each stage's card shows for it. Kept per session inside
//! the draft (so it is parked and restored with it), never persisted. Every op returns a new
//! state; nothing here talks to a server. Pipelines and counts are user data: the `Debug` of
//! every type here names variants and sizes only.
use super::aggregate_counts_plan::{CountPlan, NotCounted, PlanItem};
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{COUNT_CAP, Stage};
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use std::fmt;
use std::sync::Arc;

/// The count at which the server stopped looking: a result equal to it means "at least".
pub const CAP: u64 = COUNT_CAP as u64;

/// What one stage's card says about its count.
#[derive(Clone, PartialEq)]
pub enum StageCount {
    Exact(u64),
    /// The cap was reached: there may be more.
    AtLeast(u64),
    /// The server (or the connection) failed for this stage.
    Failed(CoreError),
    NotCounted(NotCounted),
    /// The server stopped the count at its time limit.
    TimedOut,
}

impl StageCount {
    /// The card's content for the server's answer to a count.
    pub fn of(outcome: Result<u64, CoreError>) -> Self {
        match outcome {
            Ok(n) if n >= CAP => Self::AtLeast(n),
            Ok(n) => Self::Exact(n),
            Err(CoreError::Database(text)) if text == COMMAND_TIMEOUT_MESSAGE => Self::TimedOut,
            Err(error) => Self::Failed(error),
        }
    }
}

impl fmt::Debug for StageCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exact(_) => f.write_str("Exact"),
            Self::AtLeast(_) => f.write_str("AtLeast"),
            Self::Failed(error) => write!(f, "Failed({})", error.i18n_key()),
            Self::NotCounted(reason) => write!(f, "NotCounted({reason:?})"),
            Self::TimedOut => f.write_str("TimedOut"),
        }
    }
}

/// A stage waiting for its turn.
struct Queued {
    index: usize,
    key: u64,
    pipeline: Vec<Document>,
}

/// The count in flight and those behind it.
#[derive(Clone)]
struct Running {
    /// The request number awaited, and the stage and card it counts.
    request: u64,
    index: usize,
    key: u64,
    queue: Arc<[Queued]>,
    /// The queue position of the next stage to send.
    next: usize,
}

#[derive(Clone, Default)]
enum Run {
    #[default]
    Idle,
    Running(Running),
}

/// A count to send: its request number, the stage it counts and the pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct CountSend {
    pub request: u64,
    pub stage_index: usize,
    pub pipeline: Vec<Document>,
}

/// Where a card stands in a running sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Counting,
    Waiting,
}

/// One session's counts.
#[derive(Clone, Default)]
pub struct CountsUi {
    run: Run,
    /// What each counted card shows, by the stage's identity.
    results: Arc<[(u64, StageCount)]>,
    /// The stages the results were counted for; a draft that differs now makes them out of date.
    counted_for: Option<Arc<[Stage]>>,
    /// The newest request number given out; never reused, so a late answer never matches a newer
    /// sequence.
    request: u64,
}

impl PartialEq for CountsUi {
    fn eq(&self, other: &Self) -> bool {
        self.request == other.request
            && self.results == other.results
            && self.counted_for == other.counted_for
            && self.running_request() == other.running_request()
    }
}

impl fmt::Debug for CountsUi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CountsUi")
            .field("running", &self.is_running())
            .field("results", &self.results)
            .field("request", &self.request)
            .finish()
    }
}

impl CountsUi {
    fn running_request(&self) -> Option<u64> {
        match &self.run {
            Run::Running(r) => Some(r.request),
            Run::Idle => None,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.run, Run::Running(_))
    }

    /// Whether `request` is the count awaited.
    pub fn awaits(&self, request: u64) -> bool {
        self.running_request() == Some(request)
    }

    /// The stage the count under `request` is for, while it is the one awaited.
    pub fn awaited_stage(&self, request: u64) -> Option<usize> {
        match &self.run {
            Run::Running(r) if r.request == request => Some(r.index),
            _ => None,
        }
    }

    /// A sequence for `plan` over `stages` (with their identities `keys`) starts: the old
    /// results are dropped, the stages that cannot be counted say so at once, and the first
    /// count is sent. A sequence that is running is not restarted.
    pub fn begun(
        self,
        plan: CountPlan,
        stages: &[Stage],
        keys: &[u64],
    ) -> (Self, Option<CountSend>) {
        if self.is_running() {
            return (self, None);
        }
        let key_of = |index: usize| keys.get(index).copied().unwrap_or(index as u64);
        let mut skipped = Vec::new();
        let mut queue = Vec::new();
        for item in plan.items {
            match item {
                PlanItem::Skip { index, reason } => {
                    skipped.push((key_of(index), StageCount::NotCounted(reason)));
                }
                PlanItem::Send { index, pipeline } => queue.push(Queued {
                    index,
                    key: key_of(index),
                    pipeline,
                }),
            }
        }
        let fresh = Self {
            run: Run::Idle,
            results: skipped.into(),
            counted_for: Some(stages.into()),
            ..self
        };
        fresh.sent_from(queue.into(), 0)
    }

    /// The answer to the count of stage `stage_index` under `request`; any other is stale and
    /// changes nothing. The next stage's count is sent, or the sequence ends.
    pub fn answered(
        self,
        request: u64,
        stage_index: usize,
        outcome: Result<u64, CoreError>,
    ) -> (Self, Option<CountSend>) {
        let Run::Running(running) = &self.run else {
            return (self, None);
        };
        if running.request != request || running.index != stage_index {
            return (self, None);
        }
        let (queue, next, key) = (running.queue.clone(), running.next, running.key);
        let results = self
            .results
            .iter()
            .cloned()
            .chain([(key, StageCount::of(outcome))])
            .collect();
        Self {
            run: Run::Idle,
            results,
            ..self
        }
        .sent_from(queue, next)
    }

    /// Sends the queue's entry at `next`, if any; without one the sequence is over.
    fn sent_from(self, queue: Arc<[Queued]>, next: usize) -> (Self, Option<CountSend>) {
        let Some(entry) = queue.get(next) else {
            return (self, None);
        };
        let request = self.request.wrapping_add(1);
        let send = CountSend {
            request,
            stage_index: entry.index,
            pipeline: entry.pipeline.clone(),
        };
        let run = Run::Running(Running {
            request,
            index: entry.index,
            key: entry.key,
            queue,
            next: next + 1,
        });
        (
            Self {
                run,
                request,
                ..self
            },
            Some(send),
        )
    }

    /// The count awaited under `request` gave up without an answer (its connection was replaced,
    /// or it was lost): the sequence ends, the results so far stay.
    pub fn given_up(self, request: u64) -> Self {
        match self.awaits(request) {
            true => self.cancelled(),
            false => self,
        }
    }

    /// The sequence ends (Cancel, an edit, a session switch): nothing is awaited any more and an
    /// answer still on its way is ignored; the results so far stay.
    pub fn cancelled(self) -> Self {
        match self.run {
            Run::Idle => self,
            Run::Running(_) => Self {
                run: Run::Idle,
                ..self
            },
        }
    }

    /// The counts no longer apply (another collection, a new connection): nothing is shown.
    pub fn reset(self) -> Self {
        Self {
            run: Run::Idle,
            results: Arc::from([]),
            counted_for: None,
            ..self
        }
    }

    /// What the card of the stage with identity `key` shows.
    pub fn result_of(&self, key: u64) -> Option<&StageCount> {
        self.results.iter().find(|(k, _)| *k == key).map(|(_, c)| c)
    }

    /// Whether the card at stage `index` is being counted or is waiting for its turn.
    pub fn progress_of(&self, index: usize) -> Option<Progress> {
        let Run::Running(r) = &self.run else {
            return None;
        };
        match (
            r.index == index,
            r.queue.iter().skip(r.next).any(|q| q.index == index),
        ) {
            (true, _) => Some(Progress::Counting),
            (false, true) => Some(Progress::Waiting),
            (false, false) => None,
        }
    }

    /// Whether both hold the very same results (a repaint or an ordinary action copies none).
    pub fn shares_results_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.results, &other.results)
    }

    /// Whether the counts shown were made for stages other than `stages`.
    pub fn is_out_of_date(&self, stages: &[Stage]) -> bool {
        self.counted_for.as_ref().is_some_and(|c| **c != *stages)
    }
}

#[cfg(test)]
#[path = "aggregate_counts_tests.rs"]
mod tests;
