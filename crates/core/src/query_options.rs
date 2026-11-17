//! The options of a Browse find: collation, hint, `maxTimeMS`, comment, read preference and
//! `allowDiskUse`. One type, one validation and one serde with an aggregate's options, carried
//! inside [`crate::store::Query`] so paging, re-fetches, the verdict and the guard keep them.
//!
//! What each find operation sends is decided here, once, by [`PipelineOptions::find_applied`];
//! the store never builds driver options any other way. Option values are user data: nothing
//! here logs one.

use std::time::Duration;

use crate::pipeline_options::PipelineOptions;
use crate::pipeline_options_apply::{AppliedOptions, MaxTimeEffect};

/// The options of a Browse find: the same type, rules and serde as an aggregate's.
pub type QueryOptions = PipelineOptions;

/// The find's own bound: the client-side command timeout (a test pins it to `COMMAND_TIMEOUT`).
pub const FIND_MAX_TIME_MS: u64 = 30_000;

/// The server-side bound of the verdict's and the guard's explains (today's value).
pub const FIND_EXPLAIN_MAX_TIME_MS: u64 = 2_000;

/// An operation that sends a query's find to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindOperation {
    /// The page, paging and every re-fetch.
    Find,
    /// The automatic verdict's `executionStats` explain.
    Explain,
    /// The guard's `queryPlanner` explain.
    PlanCheck,
}

impl FindOperation {
    /// The bound the operation has without any option.
    pub fn default_max_time_ms(self) -> u64 {
        match self {
            Self::Find => FIND_MAX_TIME_MS,
            Self::Explain | Self::PlanCheck => FIND_EXPLAIN_MAX_TIME_MS,
        }
    }
}

/// What one find operation sends. Built only by [`PipelineOptions::find_applied`]; the fields are
/// crate-private, so no other crate can build one that breaks the policy.
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedFindOptions {
    pub(crate) options: AppliedOptions,
    /// Whether a server-side time limit is sent (the find: only when the person set one).
    pub(crate) send_max_time: bool,
    op: FindOperation,
}

impl PipelineOptions {
    /// What `op` does with the time limit set here.
    pub fn find_max_time_effect(&self, op: FindOperation) -> MaxTimeEffect {
        let bound = op.default_max_time_ms();
        match self.max_time_ms {
            Some(set) if set < bound => MaxTimeEffect::Lowered,
            Some(set) if set > bound => MaxTimeEffect::Capped,
            _ => MaxTimeEffect::Unchanged,
        }
    }

    /// The options `op` sends: everything as set, the time limit only ever lowered.
    pub fn find_applied(&self, op: FindOperation) -> AppliedFindOptions {
        let default_ms = op.default_max_time_ms();
        // At least 1 ms: a zero would reach the server as "no limit".
        let ms = self
            .max_time_ms
            .map_or(default_ms, |set| set.clamp(1, default_ms));
        AppliedFindOptions {
            options: AppliedOptions {
                max_time: Duration::from_millis(ms),
                collation: self.collation.clone(),
                hint: self.hint.clone(),
                allow_disk_use: self.allow_disk_use,
                comment: self.comment.clone(),
                read_preference: self.read_preference,
            },
            send_max_time: op != FindOperation::Find || self.max_time_ms.is_some(),
            op,
        }
    }
}

impl AppliedFindOptions {
    /// Nothing of the person's: the operation sends exactly what it sent before options existed.
    pub fn is_default(&self) -> bool {
        let o = &self.options;
        let default_bound = Duration::from_millis(self.op.default_max_time_ms());
        o.collation.is_none()
            && o.hint.is_none()
            && o.allow_disk_use.is_none()
            && o.comment.is_none()
            && o.read_preference.is_none()
            && (self.op != FindOperation::Find || !self.send_max_time)
            && o.max_time == default_bound
    }

    /// The time limit sent is below the operation's own bound (for the timeout wording).
    pub fn max_time_lowered(&self) -> bool {
        self.send_max_time
            && self.options.max_time < Duration::from_millis(self.op.default_max_time_ms())
    }
}

#[cfg(test)]
#[path = "query_options_tests.rs"]
mod tests;
