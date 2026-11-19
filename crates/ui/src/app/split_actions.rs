//! Split view: the active tab beside one partner (#159, slice 3).
//!
//! The panes are the active tab (`Split::primary`) and its partner (`Split::secondary`). Focus
//! moves between them by swapping the two ids; where each is drawn does not change, so nothing
//! moves on screen. The split never names a tab that is gone, and a pane that becomes visible goes
//! through the one step every shown tab goes through (`tab_shown::shown_now`): a stale partner
//! reads again what it shows, a dormant one sends nothing and shows its Run.
use super::reducer::{Input, Out};
use super::reducer_dispatch::dispatch;
use super::state::AppState;
use super::tab_actions::{TabAction, activate, duplicate};
use super::tab_shown::shown_now;
use super::tab_strip::Split;
use super::tabs::TabId;
use crate::viewmodel::split_layout::{DEFAULT_RATIO, Orientation, clamped};

/// What the person does with the split. Every other tab action is not for this module.
pub(super) fn on_action(state: AppState, action: TabAction) -> Out {
    match action {
        TabAction::ToggleSplit => toggle(state),
        TabAction::OpenInOtherPane(tab) => open_in_other_pane(state, tab),
        TabAction::FocusOtherPane | TabAction::ShowOtherPane => focus_other(state),
        TabAction::SetRatio(ratio) => (
            reshaped(state, |s| Split {
                ratio: clamped(ratio),
                ..s
            }),
            Vec::new(),
        ),
        TabAction::SetOrientation(orientation) => {
            (reshaped(state, |s| Split { orientation, ..s }), Vec::new())
        }
        TabAction::PaneShown(tab) => pane_shown(state, tab),
        _ => (state, Vec::new()),
    }
}

/// What a pane did: its tab becomes the active one, then the input is applied to it. A tab that
/// has closed since the frame was drawn drops its input.
#[inline(never)]
pub(super) fn in_tab(state: AppState, tab: TabId, input: Box<Input>) -> Out {
    if !state.tabs.owner.contains_key(&tab) {
        return (state, Vec::new());
    }
    let (state, mut commands) = match state.active_tab() == Some(tab) {
        true => (state, Vec::new()),
        false => activate(state, tab),
    };
    let (state, more) = dispatch(state, *input);
    commands.extend(more);
    (state, commands)
}

/// What a pane did on its own: applied in its tab (and its connection) with the tab left where it
/// is, so a pane that refreshes itself never takes the focus from the one being used.
#[inline(never)]
pub(super) fn for_tab(state: AppState, tab: TabId, input: Box<Input>) -> Out {
    state.with_tab(tab, |inner| dispatch(inner, *input))
}

/// [`reconciled`] in place, for the end of every input: no split returns at once, and nothing of
/// the state is moved by value into the caller's frame (this is on the deepest event path).
#[inline(never)]
pub(super) fn reconcile(state: &mut AppState) {
    if state.tabs.split.is_some() {
        *state = reconciled(std::mem::take(state));
    }
}

/// The split brought in line with the tab being looked at, which any path may have changed (a
/// tab switch, a session picked in the tree, a close): the active tab is always a pane, the
/// partner it replaced stays, and the roles swap when the partner was the one activated. A split
/// whose pane is gone or whose partner holds nothing ends.
pub(super) fn reconciled(state: AppState) -> AppState {
    let Some(split) = state.tabs.split.clone() else {
        return state;
    };
    let open = |tab: &TabId| state.tabs.owner.contains_key(tab);
    if !open(&split.primary) || !open(&split.secondary) {
        return ended(state);
    }
    let next = match state.active_tab() {
        Some(now) if now == split.secondary => Split {
            primary: now,
            secondary: split.primary,
            secondary_first: !split.secondary_first,
            ..split
        },
        Some(now) if now != split.primary => Split {
            primary: now,
            ..split
        },
        _ => split,
    };
    // A pane with nothing in it (its collection was dropped) is no pane.
    if state
        .glance(next.secondary)
        .is_some_and(|g| g.is_idle_empty())
    {
        return ended(state);
    }
    with_split(state, Some(next))
}

fn with_split(state: AppState, split: Option<Split>) -> AppState {
    AppState {
        tabs: state.tabs.clone().with_split(split),
        ..state
    }
}

fn ended(state: AppState) -> AppState {
    with_split(state, None)
}

fn reshaped(state: AppState, change: impl FnOnce(Split) -> Split) -> AppState {
    let split = state.tabs.split.clone().map(change);
    match split {
        Some(_) => with_split(state, split),
        None => state,
    }
}

fn toggle(state: AppState) -> Out {
    if state.tabs.split.is_some() {
        return (ended(state), Vec::new());
    }
    let Some(active) = state.active_tab() else {
        return (state, Vec::new());
    };
    match partner_for(&state, active) {
        Some(partner) => paired(state, active, partner),
        None => copy_beside(state, active),
    }
}

/// The tab used most recently before `active`, among the tabs the strip shows.
fn partner_for(state: &AppState, active: TabId) -> Option<TabId> {
    let visible = state.visible_tabs();
    state
        .tabs
        .mru
        .iter()
        .copied()
        .find(|tab| *tab != active && visible.contains(tab))
}

fn open_in_other_pane(state: AppState, tab: TabId) -> Out {
    let Some(active) = state.active_tab() else {
        return (state, Vec::new());
    };
    match (state.tabs.owner.contains_key(&tab), tab == active) {
        (false, _) => (state, Vec::new()),
        (true, true) => copy_beside(state, tab),
        (true, false) => paired(state, active, tab),
    }
}

/// `primary` (the active tab) and `secondary` as the panes; the divider and the orientation the
/// person chose are kept when the view was already split. The partner is shown now.
fn paired(state: AppState, primary: TabId, secondary: TabId) -> Out {
    let split = match state.tabs.split.clone() {
        Some(kept) => Split {
            primary,
            secondary,
            ..kept
        },
        None => Split {
            primary,
            secondary,
            ratio: DEFAULT_RATIO,
            orientation: Orientation::default(),
            secondary_first: false,
        },
    };
    pane_shown(with_split(state, Some(split)), secondary)
}

/// Only one tab (or only `tab` to show): a copy of it opens in the other pane. The copy runs its
/// query once, as any duplicate does; the original stays where it was on screen.
fn copy_beside(state: AppState, tab: TabId) -> Out {
    let (state, commands) = duplicate(state, tab);
    let Some(copy) = state.active_tab().filter(|copy| *copy != tab) else {
        // The limit notice says why nothing opened (or the tab holds nothing to copy).
        return (state, commands);
    };
    let split = Split {
        primary: copy,
        secondary: tab,
        ratio: state
            .tabs
            .split
            .as_ref()
            .map_or(DEFAULT_RATIO, |kept| kept.ratio),
        orientation: state
            .tabs
            .split
            .as_ref()
            .map_or(Orientation::default(), |kept| kept.orientation),
        secondary_first: true,
    };
    (with_split(state, Some(split)), commands)
}

fn focus_other(state: AppState) -> Out {
    match state.tabs.split.as_ref().map(|s| s.secondary) {
        Some(other) => activate(state, other),
        None => (state, Vec::new()),
    }
}

/// The partner is on screen: it reads again what it shows when it is stale, and sends nothing
/// when it is dormant (its Run is on screen instead). Any other tab is not a pane: nothing.
fn pane_shown(state: AppState, tab: TabId) -> Out {
    match state.tabs.split.as_ref().map(|s| s.secondary) == Some(tab) {
        true => state.with_tab_shown(tab, shown_now),
        false => (state, Vec::new()),
    }
}

#[cfg(test)]
#[path = "split_actions_tests.rs"]
mod tests;
