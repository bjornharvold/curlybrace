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
    // The split's partner is on screen beside the active tab: it keeps its page, and does not
    // count against the parked ones.
    let partner = state.tabs.split.as_ref().map(|s| s.secondary);
    let mut holding: Vec<(TabId, usize)> = state
        .parked_tabs
        .iter()
        .filter(|(tab, view)| view.page.is_some() && Some(**tab) != partner)
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

/// Closes every tab that shows nothing, holds no work and is not the one being looked at, unless
/// it is its session's only tab: nobody can see such a tab, so nothing else would ever close it
/// (a Cmd+T tab left behind by a click in another connection's tree, a tab blanked by a vanished
/// database or a dropped collection). Called after every reducer step; with one tab per session
/// (the usual case) it returns at once.
pub(super) fn sweep_empty(state: &mut AppState) {
    if state.tabs.order.len() <= state.open.len() {
        return;
    }
    let active = state.active_tab();
    let doomed: Vec<TabId> = state
        .tabs
        .order
        .iter()
        .copied()
        .filter(|t| Some(*t) != active)
        .filter(|t| state.glance(*t).is_some_and(|g| g.is_idle_empty()))
        .collect();
    for tab in doomed {
        let lone = state
            .tabs
            .owner
            .get(&tab)
            .is_none_or(|session| state.tabs.of_session(*session).len() < 2);
        if !lone {
            *state = std::mem::take(state).tab_closed(tab);
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
    // A draft that waits for an answer (a preview, an explain, a count, a run) is left alone: the
    // answer is routed to it by number. Otherwise its preview goes back to Idle and its explain
    // and counts are reset, but a finished run keeps its outcome (what the person last did).
    let waiting = view.aggregate.in_flight.is_some()
        || view.aggregate.explain.is_loading()
        || view.aggregate.counts.is_running()
        || view.aggregate.run.is_running();
    let aggregate = match waiting {
        true => view.aggregate,
        false => {
            let run = view.aggregate.run.clone().dialog_closed();
            Box::new(view.aggregate.on_collection_changed().with_run(run))
        }
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
