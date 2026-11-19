//! Bounds on the pages parked tabs keep (#159).
//!
//! The tab being looked at is in the foreground and never in `parked_tabs`, so it is never
//! touched. A parked tab past the bound loses its page, its Aggregate preview and its index chart
//! series, keeps its query and text, and is marked stale: it reads the same query again when it
//! is shown.
use super::state::AppState;
use super::tabs::{Freshness, TabId, TabView};
use crate::viewmodel::IndexesState;

/// The parked tabs that keep their page, most recently used first.
pub const RETAINED_PARKED: usize = 6;
/// The most the kept pages of parked tabs hold together.
pub const RETAINED_BYTES: usize = 64 * 1024 * 1024;

/// Applies the bounds in place (this runs after every reducer step).
pub(super) fn evict(state: &mut AppState) {
    evict_within(state, RETAINED_PARKED, RETAINED_BYTES);
}

/// The pages of parked tabs kept: at most `parked` of them, most recently used first, holding at
/// most `bytes` together.
pub(super) fn evict_within(state: &mut AppState, parked: usize, bytes: usize) {
    if state.parked_tabs.is_empty() {
        return;
    }
    let mut holding: Vec<(TabId, usize)> = state
        .parked_tabs
        .iter()
        .filter(|(_, view)| view.page.is_some())
        .map(|(tab, view)| (*tab, view.page_key.map_or(0, |key| key.bytes)))
        .collect();
    if holding.is_empty() {
        return;
    }
    // Most recently used first; a tab the strip has no recency for is the oldest.
    let rank = |tab: &TabId| {
        state
            .tabs
            .mru
            .iter()
            .position(|t| t == tab)
            .unwrap_or(usize::MAX)
    };
    holding.sort_by_key(|(tab, _)| rank(tab));
    let mut kept = 0usize;
    let mut held = 0usize;
    for (tab, size) in holding {
        let fits = kept < parked && held.saturating_add(size) <= bytes;
        match fits {
            true => {
                kept += 1;
                held += size;
            }
            false => evict_tab(state, tab),
        }
    }
}

fn evict_tab(state: &mut AppState, tab: TabId) {
    if let Some(view) = state.parked_tabs.remove(&tab) {
        state.parked_tabs.insert(tab, Box::new(evicted(*view)));
    }
}

/// `view` without what the server can supply again.
fn evicted(view: TabView) -> TabView {
    // A preview in flight is awaited by number; leave a draft that waits for an answer alone.
    let aggregate = match view.aggregate.in_flight.is_some() {
        true => view.aggregate,
        false => Box::new(view.aggregate.on_collection_changed()),
    };
    TabView {
        page: None,
        page_key: None,
        tree: view.tree.results_replaced(),
        aggregate,
        indexes: IndexesState {
            series: Vec::new(),
            ..view.indexes
        },
        freshness: Freshness::Stale,
        ..view
    }
}

#[cfg(test)]
#[path = "tab_eviction_tests.rs"]
mod tests;
