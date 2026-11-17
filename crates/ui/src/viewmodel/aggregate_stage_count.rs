//! What one stage's card says about its count. Counts and failures are user data: the `Debug`
//! names variants only.
use super::aggregate_counts_plan::NotCounted;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::COUNT_CAP;
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use std::fmt;

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
