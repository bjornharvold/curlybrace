//! Index-miss detection: turns a server explain document into a typed [`Verdict`]
//! (`classify`) and proposes an index for the query (`suggest`).

mod classify;
#[cfg(test)]
mod classify_sharded_tests;
#[cfg(test)]
mod classify_tests;
mod suggest;
#[cfg(test)]
mod suggest_tests;
mod verdict;

pub use classify::classify;
pub use suggest::suggest;
pub use verdict::{Reason, SuggestedIndex, Verdict, VerdictKind};
