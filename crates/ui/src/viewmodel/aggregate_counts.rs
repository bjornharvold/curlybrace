//! The per-stage counts of the Aggregate tab: one bounded count per enabled stage, sent one after
//! another (never in parallel), and what each stage's card shows for it. Kept per session inside
//! the draft (so it is parked and restored with it), never persisted. Every op returns a new
//! state; nothing here talks to a server. Pipelines and counts are user data: the `Debug` of
//! every type here names variants and sizes only.
use super::aggregate_counts_plan::{CountPlan, PlanItem};
pub use super::aggregate_stage_count::{CAP, StageCount};
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{PipelineOptions, Stage};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The request number the next count gets, for the whole process: a view that is dropped and
/// built again (a closed session opened anew) starts with no memory of the numbers it gave out,
/// and a late answer of one of them must never match a count of the new view.
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

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
    /// A start held back behind the count still out on the server (see `CountsUi::lingering`):
    /// its first count goes out when that one is released.
    Held(Arc<[Queued]>),
}

/// A count to send: its request number, the stage it counts and the pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct CountSend {
    pub request: u64,
    pub stage_index: usize,
    pub pipeline: Vec<Document>,
    /// The options the sequence began with, whatever the draft says by now.
    pub options: PipelineOptions,
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
    /// The options the sequence began with and the results were counted with: every count of
    /// the sequence sends these, and options that differ now make the results out of date.
    options: Arc<PipelineOptions>,
    /// The newest request number given out; numbers come from one process-wide counter, so one is
    /// never reused, not even by another view of the same session.
    request: u64,
    /// A count that was cancelled (or whose sequence an edit ended) and may still be running on
    /// the server. One count at a time holds even then: a new start is held back until this
    /// request is released by its answer or failure, or by a loss or a reset.
    lingering: Option<u64>,
}

impl PartialEq for CountsUi {
    fn eq(&self, other: &Self) -> bool {
        self.request == other.request
            && self.results == other.results
            && self.counted_for == other.counted_for
            && self.options == other.options
            && self.running_request() == other.running_request()
            && self.lingering == other.lingering
            && self.is_running() == other.is_running()
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
            Run::Idle | Run::Held(_) => None,
        }
    }

    pub fn is_running(&self) -> bool {
        !matches!(self.run, Run::Idle)
    }

    /// Whether `request` is the count awaited.
    pub fn awaits(&self, request: u64) -> bool {
        self.running_request() == Some(request)
    }

    /// Whether `request` is a cancelled count still out on the server.
    pub fn lingers(&self, request: u64) -> bool {
        self.lingering == Some(request)
    }

    /// The stage the count under `request` is for, while it is the one awaited.
    pub fn awaited_stage(&self, request: u64) -> Option<usize> {
        match &self.run {
            Run::Running(r) if r.request == request => Some(r.index),
            _ => None,
        }
    }

    /// A sequence for `plan` over `stages` (with their identities `keys`) and the draft's
    /// `options` starts: the old results are dropped, the stages that cannot be counted say so
    /// at once, and the first count is sent, unless a cancelled count is still out: then the start waits for it (every
    /// card says so) and `released` sends the first count. A sequence that is running (or
    /// waiting) is not restarted.
    pub fn begun(
        self,
        plan: CountPlan,
        stages: &[Stage],
        keys: &[u64],
        options: &PipelineOptions,
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
            options: Arc::new(options.clone()),
            ..self
        };
        let queue: Arc<[Queued]> = queue.into();
        match fresh.lingering {
            Some(_) => (
                Self {
                    run: Run::Held(queue),
                    ..fresh
                },
                None,
            ),
            None => fresh.sent_from(queue, 0),
        }
    }

    /// The answer or failure of `request` arrived, whatever it says: when it is the cancelled
    /// count still out, the slot is free and a held start sends its first count. Never a result.
    pub fn released(self, request: u64) -> (Self, Option<CountSend>) {
        if self.lingering != Some(request) {
            return (self, None);
        }
        let freed = Self {
            lingering: None,
            ..self
        };
        match &freed.run {
            Run::Held(queue) => {
                let queue = queue.clone();
                freed.sent_from(queue, 0)
            }
            _ => (freed, None),
        }
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
        let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let send = CountSend {
            request,
            stage_index: entry.index,
            pipeline: entry.pipeline.clone(),
            options: (*self.options).clone(),
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

    /// The count under `request` gave up without an answer (its connection was replaced, or it
    /// was lost, or it could not be sent): nothing is out for it any more. A sequence awaiting it
    /// ends and so does a start held behind it; the results so far stay.
    pub fn given_up(self, request: u64) -> Self {
        match (self.awaits(request), self.lingering == Some(request)) {
            (true, _) => Self {
                run: Run::Idle,
                ..self
            },
            (_, true) => Self {
                run: Run::Idle,
                lingering: None,
                ..self
            },
            _ => self,
        }
    }

    /// The sequence ends (Cancel, an edit, a session switch): nothing is awaited any more and an
    /// answer still on its way is ignored; the results so far stay. The count in flight may
    /// still be running on the server: it is remembered (`lingering`) so a new start waits for it.
    pub fn cancelled(self) -> Self {
        match &self.run {
            Run::Idle => self,
            Run::Held(_) => Self {
                run: Run::Idle,
                ..self
            },
            Run::Running(r) => Self {
                lingering: Some(r.request),
                run: Run::Idle,
                ..self
            },
        }
    }

    /// The counts no longer apply (another collection): nothing is shown and nothing is
    /// awaited. A count still out on the server (the one awaited, or one already cancelled) stays
    /// remembered: a new start waits for its answer, so two counts never overlap.
    pub fn reset(self) -> Self {
        let out = self.cancelled();
        Self {
            run: Run::Idle,
            results: Arc::from([]),
            counted_for: None,
            ..out
        }
    }

    /// The connection is gone or replaced: nothing is out on it any more, so nothing is waited
    /// for either.
    pub fn connection_lost(self) -> Self {
        Self {
            lingering: None,
            ..self.reset()
        }
    }

    /// What the card of the stage with identity `key` shows.
    pub fn result_of(&self, key: u64) -> Option<&StageCount> {
        self.results.iter().find(|(k, _)| *k == key).map(|(_, c)| c)
    }

    /// Whether the card at stage `index` is being counted or is waiting for its turn.
    pub fn progress_of(&self, index: usize) -> Option<Progress> {
        let r = match &self.run {
            Run::Running(r) => r,
            Run::Held(queue) => {
                return queue
                    .iter()
                    .any(|q| q.index == index)
                    .then_some(Progress::Waiting);
            }
            Run::Idle => return None,
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

    /// Whether the counts shown were made for stages or options other than these.
    pub fn is_out_of_date(&self, stages: &[Stage], options: &PipelineOptions) -> bool {
        self.counted_for
            .as_ref()
            .is_some_and(|c| **c != *stages || *self.options != *options)
    }
}

#[cfg(test)]
#[path = "aggregate_counts_tests.rs"]
mod tests;
