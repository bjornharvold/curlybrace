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

/// What an operation did with the time limit the person set, so the page can say so: a limit
/// can only lower an operation's own bound, never raise it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaxTimeEffect {
    /// Nothing set, or the value equals the operation's own bound.
    Unchanged,
    /// The value is below the bound and is the one sent.
    Lowered,
    /// The value is above the bound and was cut down to it ("previews stop at 10 s").
    Capped,
}

/// What one operation sends. Built only by [`PipelineOptions::applied`] or
/// [`AppliedOptions::bounded`]; the fields are crate-private, so no other crate can build one that
/// breaks the rules of the policy (the store re-enforces the two safety rules regardless).
#[derive(Clone, PartialEq)]
pub struct AppliedOptions {
    pub(crate) max_time: Duration,
    pub(crate) collation: Option<CollationOption>,
    pub(crate) hint: Option<HintOption>,
    /// Preview and count: always `Some(false)`.
    pub(crate) allow_disk_use: Option<bool>,
    pub(crate) comment: Option<String>,
    /// The run: always `None` (the store pins the primary).
    pub(crate) read_preference: Option<ReadPreferenceMode>,
}

/// Shows which options are set, never their values: a comment or a hint is the person's text and
/// must not reach a log through `{:?}`.
impl std::fmt::Debug for AppliedOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppliedOptions")
            .field("max_time_ms", &self.max_time.as_millis())
            .field("has_collation", &self.collation.is_some())
            .field("has_hint", &self.hint.is_some())
            .field("allow_disk_use", &self.allow_disk_use)
            .field("has_comment", &self.comment.is_some())
            .field("has_read_preference", &self.read_preference.is_some())
            .finish()
    }
}

impl PipelineOptions {
    /// What `op` does with the time limit set here.
    pub fn max_time_effect(&self, op: Operation) -> MaxTimeEffect {
        let bound = op.default_max_time_ms();
        match self.max_time_ms {
            Some(set) if set < bound => MaxTimeEffect::Lowered,
            Some(set) if set > bound => MaxTimeEffect::Capped,
            _ => MaxTimeEffect::Unchanged,
        }
    }

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
