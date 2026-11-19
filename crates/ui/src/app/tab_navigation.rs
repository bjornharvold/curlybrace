//! Moving between and reordering the tabs the strip shows (#159): next and previous, most
//! recently used, by position, and moving a tab along the strip.
use super::reducer::Out;
use super::state::AppState;
use super::tab_actions::activate;
use super::tabs::TabId;

/// The tab being looked at, `by` places along the strip (stopping at either end).
pub(super) fn moved_by(state: AppState, by: i32) -> AppState {
    let visible = state.visible_tabs();
    let from = state
        .active_tab()
        .and_then(|t| visible.iter().position(|v| *v == t).map(|p| (t, p)));
    let Some((tab, at)) = from else {
        return state;
    };
    let to = (at as i64 + i64::from(by)).clamp(0, visible.len() as i64 - 1) as usize;
    moved(state, tab, to)
}

/// `tab` moved to the place of the `to`-th tab the strip shows.
pub(super) fn moved(state: AppState, tab: TabId, to: usize) -> AppState {
    let visible = state.visible_tabs();
    let target = visible.get(to).or_else(|| visible.last()).copied();
    let place = target.and_then(|t| state.tabs.order.iter().position(|o| *o == t));
    match place {
        Some(place) => AppState {
            tabs: state.tabs.clone().moved(tab, place),
            ..state
        },
        None => state,
    }
}

/// The tab `by` places from the one being looked at, in the order the strip shows them.
pub(super) fn step(state: AppState, by: isize) -> Out {
    match state.visible_neighbour(state.active_tab(), by) {
        Some(tab) => activate(state, tab),
        None => (state, Vec::new()),
    }
}

/// The previous (or, `oldest`, the least recently used) tab other than the one being looked at.
pub(super) fn mru(state: AppState, oldest: bool) -> Out {
    let visible = state.visible_tabs();
    let active = state.active_tab();
    let recent: Vec<TabId> = state
        .tabs
        .mru
        .iter()
        .copied()
        .filter(|t| Some(*t) != active && visible.contains(t))
        .collect();
    let pick = match oldest {
        true => recent.last(),
        false => recent.first(),
    };
    match pick.copied() {
        Some(tab) => activate(state, tab),
        None => (state, Vec::new()),
    }
}

pub(super) fn nth(state: AppState, at: usize) -> Out {
    match state.visible_tabs().get(at).copied() {
        Some(tab) => activate(state, tab),
        None => (state, Vec::new()),
    }
}

pub(super) fn last(state: AppState) -> Out {
    match state.visible_tabs().last().copied() {
        Some(tab) => activate(state, tab),
        None => (state, Vec::new()),
    }
}
