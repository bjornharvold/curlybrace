//! The one source of the numbers a tab's reads carry: unique for the whole process, so an answer
//! names exactly one waiting tab (see `tabs::routing`). The same rule as the counts' own source in
//! `viewmodel::aggregate_counts`, which now draws from here too.
//!
//! It is a static rather than a field of `AppState` because the numbers are taken inside pure
//! viewmodel methods that have no access to the state, and threading a counter through them would
//! change a dozen signatures. Numbers only grow, so a newer request always has a larger one.
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// The next request number: never `0`, never repeated while the process runs.
pub fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
#[path = "request_numbers_tests.rs"]
mod tests;
