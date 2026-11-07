//! Index-miss detection: turns a server explain document into a typed [`Verdict`]
//! (`classify`) and proposes an index for the query (`suggest`).

mod classify;
#[cfg(test)]
mod classify_sharded_tests;
#[cfg(test)]
mod classify_tests;
pub(crate) mod suggest;
#[cfg(test)]
mod suggest_tests;
mod verdict;
#[cfg(test)]
mod verdict_tests;

pub use classify::classify;
pub(crate) use classify::{count, is_inefficient};
pub use suggest::suggest;
pub use verdict::{Reason, SuggestedIndex, Verdict, VerdictKind, worst};
