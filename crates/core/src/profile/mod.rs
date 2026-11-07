//! Profiler entries: parsing a `system.profile` document, deciding whether it is our own
//! traffic and judging it with the same thresholds as the explain classifier.

mod classify;
#[cfg(test)]
mod classify_tests;
mod entry;
#[cfg(test)]
mod entry_tests;
mod filter;
#[cfg(test)]
mod filter_tests;

pub use classify::classify_entry;
pub use entry::{OWN_APP_NAME, ProfileEntry, is_own_traffic, parse_entry};
pub use filter::entry_has_filter;
