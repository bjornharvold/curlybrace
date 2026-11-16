//! What each operation sends of a pipeline's options: the one policy function. Preview and count
//! never spill to disk, the run never carries a read preference, and `maxTimeMS` only ever
//! lowers an operation's own bound. The store re-enforces the two safety rules.

use std::time::Duration;

use crate::explain::AGGREGATE_EXPLAIN_MAX_TIME_MS;
use crate::pipeline::{AGGREGATE_RUN_MAX_TIME_MS, PREVIEW_MAX_TIME_MS};
use crate::pipeline_options::{
    CollationOption, HintOption, MAX_TIME_MS_CAP, PipelineOptions, ReadPreferenceMode,
};

/// An operation that sends a pipeline to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Preview,
    Count,
    Explain,
    Run,
}

impl Operation {
    /// The server-side time limit the operation has without any option.
    pub fn default_max_time_ms(self) -> u64 {
        match self {
            Self::Preview | Self::Count => PREVIEW_MAX_TIME_MS,
            Self::Explain => AGGREGATE_EXPLAIN_MAX_TIME_MS,
            Self::Run => AGGREGATE_RUN_MAX_TIME_MS,
        }
    }

    /// Whether the operation reads on a store of the person's choosing (not a confirmed write).
    fn is_read(self) -> bool {
        !matches!(self, Self::Run)
    }

    /// Preview and count are cheap by promise: no spill to disk, whatever was set.
    fn forbids_disk_use(self) -> bool {
        matches!(self, Self::Preview | Self::Count)
    }
}

/// What one operation sends. Built only by [`PipelineOptions::applied`] or
/// [`AppliedOptions::bounded`].
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedOptions {
    pub max_time: Duration,
    pub collation: Option<CollationOption>,
    pub hint: Option<HintOption>,
    /// Preview and count: always `Some(false)`.
    pub allow_disk_use: Option<bool>,
    pub comment: Option<String>,
    /// The run: always `None` (the store pins the primary).
    pub read_preference: Option<ReadPreferenceMode>,
}

impl PipelineOptions {
    /// The options `op` sends.
    pub fn applied(&self, op: Operation) -> AppliedOptions {
        let default_ms = op.default_max_time_ms();
        // At least 1 ms: a zero would reach the server as "no limit".
        let ms = self
            .max_time_ms
            .map_or(default_ms, |set| set.clamp(1, default_ms));
        AppliedOptions {
            max_time: Duration::from_millis(ms),
            collation: self.collation.clone(),
            hint: self.hint.clone(),
            allow_disk_use: disk_use(op, self.allow_disk_use),
            comment: self.comment.clone(),
            read_preference: self.read_preference.filter(|_| op.is_read()),
        }
    }
}

fn disk_use(op: Operation, set: Option<bool>) -> Option<bool> {
    if op.forbids_disk_use() {
        Some(false)
    } else {
        set
    }
}

/// `d` within 1 ms and the cap of an option: a zero would mean "no limit" to the server and a
/// larger value would outlast every client bound. Every time limit sent goes through this.
pub(crate) fn clamped_time(d: Duration) -> Duration {
    d.clamp(
        Duration::from_millis(1),
        Duration::from_millis(MAX_TIME_MS_CAP),
    )
}

impl AppliedOptions {
    /// No option of the person's, only a time bound: for callers with their own bound.
    pub fn bounded(op: Operation, max_time: Duration) -> Self {
        Self {
            max_time: clamped_time(max_time),
            collation: None,
            hint: None,
            allow_disk_use: disk_use(op, None),
            comment: None,
            read_preference: None,
        }
    }

    /// The person's value is below the operation's default (so the status can say so).
    pub fn max_time_lowered(&self, op: Operation) -> bool {
        self.max_time < Duration::from_millis(op.default_max_time_ms())
    }
}

#[cfg(test)]
#[path = "pipeline_options_apply_tests.rs"]
mod tests;
