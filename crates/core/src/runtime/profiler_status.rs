//! What the profile poller reports on its status channel, on change only.

use super::sampler::SamplerPause;

/// The state of a connection's profile poller. The UI maps each to an i18n key.
///
/// A poller that has nothing to report is `Idle` and sends nothing at all until something else
/// happens: its first status arrives only when it differs from `Idle`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfilerStatus {
    /// No database has its profiler on (or none can be read): nothing is collected.
    Idle,
    /// Entries of `databases` databases are being read.
    Reading { databases: usize },
    /// Every database that was tried failed in a way that is not a pause (see the log for the
    /// cause); the poller retries every interval. Never `Idle`: that means nothing to read.
    Failing,
    /// The profile of `db` rolled entries off before they were read; the loss is on the record.
    /// Reported for the tick that found it; the next tick without a loss reports `Reading` again.
    MissedEntries { db: String },
    /// The poller cannot collect for the moment (same reasons as the index sampler).
    Paused(SamplerPause),
}
