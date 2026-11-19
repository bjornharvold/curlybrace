//! What the person does with tabs: open, duplicate, close, reopen, reorder, switch (#159).
//!
//! Every action ends with the tab it shows swapped into the foreground (`AppState::tab_current`),
//! so the code that reads `state.page` or `state.find` never learns that tabs exist. A tab opened
//! here runs one read, the default query of its collection; nothing else is sent unless a stale
//! tab is shown (it re-reads what it already read) or a dormant tab is told to Run.
use super::actions::{gated, issue_find};
use super::reducer::Out;
use super::state::{AppState, ConnStatus, Workspace};
use super::tab_seed::{Seed, seed_of, seeded};
use super::tab_strip::TabNotice;
use super::tabs::{Freshness, TabId};
use crate::viewmodel::{CollectionTab, submit_find};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// The most tabs the app holds at once.
pub const MAX_TABS: usize = 20;
/// How many closed tabs can be reopened.
pub const CLOSED_KEPT: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum TabAction {
    Activate(TabId),
    New,
    OpenInNew {
        session: Uuid,
        ns: Namespace,
    },
    Duplicate(TabId),
    Close(TabId),
    CloseOthers(TabId),
    CloseRight(TabId),
    ReopenClosed,
    /// Move `tab` to position `to` of the tabs the strip shows.
    Move {
        tab: TabId,
        to: usize,
    },
    Next,
    Previous,
    NextMru,
    PreviousMru,
    /// The tab at this (zero-based) position of the strip.
    Nth(usize),
    Last,
    /// A dormant tab's Run: the only way its find is sent.
    Run(TabId),
}

pub(super) fn on_action(state: AppState, action: TabAction) -> Out {
    let state = AppState {
        tabs: state.tabs.clone().with_notice(None),
        ..state
    };
    match action {
        TabAction::Activate(tab) => activate(state, tab),
        TabAction::New => new_tab(state),
        TabAction::OpenInNew { session, ns } => open_in_new(state, session, ns),
        TabAction::Duplicate(tab) => duplicate(state, tab),
        TabAction::Close(tab) => close(state, tab),
        TabAction::CloseOthers(tab) => close_many(state, tab, false),
        TabAction::CloseRight(tab) => close_many(state, tab, true),
        TabAction::ReopenClosed => reopen_closed(state),
        TabAction::Move { tab, to } => (moved(state, tab, to), Vec::new()),
        TabAction::Next => step(state, 1),
        TabAction::Previous => step(state, -1),
        TabAction::NextMru => mru(state, false),
        TabAction::PreviousMru => mru(state, true),
        TabAction::Nth(at) => nth(state, at),
        TabAction::Last => last(state),
        TabAction::Run(tab) => run(state, tab),
    }
}

/// `state` refused a new tab: the notice says why. Nothing else changes.
fn refused(state: AppState) -> Out {
    let tabs = state
        .tabs
        .clone()
        .with_notice(Some(TabNotice::LimitReached));
    (AppState { tabs, ..state }, Vec::new())
}

fn at_limit(state: &AppState) -> bool {
    state.tabs.order.len() >= MAX_TABS
}

/// The view the person leaves behind when another tab is shown: its run confirmation belongs to
/// what was on screen when it opened, and another session's counts end with it.
fn leaving_view(state: AppState, session: Uuid) -> AppState {
    let state = super::aggregate_counts_actions::left_for(state, session);
    super::aggregate_run_actions::dialog_closed(state)
}

/// Shows `tab`: its session becomes the active one and the tab its current one. Sends nothing.
fn shown(state: AppState, tab: TabId) -> AppState {
    let Some(session) = state.tabs.owner.get(&tab).copied() else {
        return state;
    };
    let state = AppState {
        workspace: Workspace::Browse,
        ..leaving_view(state, session)
    };
    state.tab_current(tab)
}

/// A new, empty tab of `session` after the tab being looked at, shown. `None` when the session
/// is not open.
fn opened_shown(state: AppState, session: Uuid) -> (AppState, Option<TabId>) {
    let after = state.active_tab();
    let (state, tab) = state.tab_opened_after(session, after);
    match tab {
        Some(tab) => (shown(state, tab), Some(tab)),
        None => (state, None),
    }
}

fn new_tab(state: AppState) -> Out {
    let Some(session) = state.active else {
        return (state, Vec::new());
    };
    if at_limit(&state) {
        return refused(state);
    }
    (opened_shown(state, session).0, Vec::new())
}

fn open_in_new(state: AppState, session: Uuid, ns: Namespace) -> Out {
    if !state.is_open(session) {
        return (state, Vec::new());
    }
    if state.status_of(session) != Some(ConnStatus::Connected) {
        return gated(state, |s| (s, Vec::new()));
    }
    if at_limit(&state) {
        return refused(state);
    }
    let (state, tab) = opened_shown(state, session);
    match tab {
        Some(_) => super::actions::sidebar(state, crate::viewmodel::SidebarAction::Open(ns)),
        None => (state, Vec::new()),
    }
}

/// Shows `tab`. A stale tab re-reads the page it shows (and its index list on Indexes), which is
/// a read it already ran; a dormant tab sends nothing.
fn activate(state: AppState, tab: TabId) -> Out {
    if !state.tabs.owner.contains_key(&tab) {
        return (state, Vec::new());
    }
    let freshness = state.freshness_of(tab);
    let session = state.tabs.owner.get(&tab).copied().unwrap_or_default();
    let reads =
        freshness == Freshness::Stale && state.status_of(session) == Some(ConnStatus::Connected);
    let state = shown(state, tab);
    match reads {
        true => refetch(refreshed(state, tab)),
        false => (state, Vec::new()),
    }
}

/// `tab` (current) is no longer marked stale.
fn refreshed(state: AppState, tab: TabId) -> AppState {
    AppState {
        tabs: state.tabs.clone().with_stale(tab, false),
        ..state
    }
}

/// The foreground tab reads its page again (and its indexes when they are the ones shown).
fn refetch(state: AppState) -> Out {
    let query = match state.page.is_none() && state.loading_page {
        true => state.issued_query.clone(),
        false => state.page_query.clone(),
    };
    let (state, mut commands) = issue_find(state, query);
    match state.tab {
        CollectionTab::Indexes => {
            let (state, more) = super::index_actions::switch_tab(state, CollectionTab::Indexes);
            commands.extend(more);
            (state, commands)
        }
        CollectionTab::Documents | CollectionTab::Aggregate => (state, commands),
    }
}

fn duplicate(state: AppState, tab: TabId) -> Out {
    let Some(session) = state.tabs.owner.get(&tab).copied() else {
        return (state, Vec::new());
    };
    if at_limit(&state) {
        return refused(state);
    }
    let (state, seed) = seed_of(state, tab);
    let Some(seed) = seed else {
        return (state, Vec::new());
    };
    let (state, new) = state.tab_opened_after(session, Some(tab));
    let Some(new) = new else {
        return (state, Vec::new());
    };
    seeded(shown(state, new), seed, new)
}

/// The record of `tab` for the closed stack, when it shows a collection.
fn remembered(state: AppState, tab: TabId) -> AppState {
    let Some(session) = state.tabs.owner.get(&tab).copied() else {
        return state;
    };
    let (state, seed) = seed_of(state, tab);
    match seed.filter(Seed::has_collection) {
        Some(seed) => AppState {
            tabs: state
                .tabs
                .clone()
                .with_closed(seed.closed(session), CLOSED_KEPT),
            ..state
        },
        None => state,
    }
}

/// `tab` closed, nothing shown in its place and nothing sent. A session's last tab is emptied
/// instead of closed: the session always has a tab to show.
fn close_one(state: AppState, tab: TabId) -> AppState {
    let Some(session) = state.tabs.owner.get(&tab).copied() else {
        return state;
    };
    let state = remembered(state, tab);
    match state.tabs.of_session(session).len() > 1 {
        true => state.tab_closed(tab),
        false => {
            let (state, _) = state.with_tab(tab, |s| (s.current_tab_emptied(), Vec::new()));
            AppState {
                tabs: state.tabs.clone().awakened(tab).with_stale(tab, false),
                ..state
            }
        }
    }
}

/// The tab shown next to `tab` among the tabs the strip shows: its right neighbour, else its
/// left one.
fn neighbour_of(state: &AppState, tab: TabId) -> Option<TabId> {
    let visible = state.visible_tabs();
    let at = visible.iter().position(|t| *t == tab)?;
    visible
        .get(at + 1)
        .or_else(|| at.checked_sub(1).and_then(|p| visible.get(p)))
        .copied()
}

fn close(state: AppState, tab: TabId) -> Out {
    let focus = (state.active_tab() == Some(tab))
        .then(|| neighbour_of(&state, tab))
        .flatten();
    let state = close_one(state, tab);
    match focus {
        Some(next) => activate(state, next),
        None => (state, Vec::new()),
    }
}

/// Closes every tab shown other than `keep` (or only those after it), then shows `keep`.
fn close_many(state: AppState, keep: TabId, right_only: bool) -> Out {
    let visible = state.visible_tabs();
    let from = match right_only {
        true => visible.iter().position(|t| *t == keep).map_or(0, |p| p + 1),
        false => 0,
    };
    let doomed: Vec<TabId> = visible
        .into_iter()
        .skip(from)
        .filter(|t| *t != keep)
        .collect();
    let state = doomed.into_iter().fold(state, close_one);
    activate(state, keep)
}

fn reopen_closed(state: AppState) -> Out {
    if at_limit(&state) {
        return refused(state);
    }
    let (tabs, closed) = state.tabs.clone().without_newest_closed();
    let state = AppState { tabs, ..state };
    let Some(closed) = closed else {
        return (state, Vec::new());
    };
    let session = closed.session;
    let (state, tab) = opened_shown(state, session);
    match tab {
        Some(tab) => seeded(state, Seed::of_closed(closed), tab),
        None => (state, Vec::new()),
    }
}

/// `tab` moved to the place of the `to`-th tab the strip shows.
fn moved(state: AppState, tab: TabId, to: usize) -> AppState {
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
fn step(state: AppState, by: isize) -> Out {
    match state.visible_neighbour(state.active_tab(), by) {
        Some(tab) => activate(state, tab),
        None => (state, Vec::new()),
    }
}

/// The previous (or, `oldest`, the least recently used) tab other than the one being looked at.
fn mru(state: AppState, oldest: bool) -> Out {
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

fn nth(state: AppState, at: usize) -> Out {
    match state.visible_tabs().get(at).copied() {
        Some(tab) => activate(state, tab),
        None => (state, Vec::new()),
    }
}

fn last(state: AppState) -> Out {
    match state.visible_tabs().last().copied() {
        Some(tab) => activate(state, tab),
        None => (state, Vec::new()),
    }
}

/// A dormant tab's Run: the tab is shown and its find bar's query is sent, which is the one thing
/// that wakes it. A query the find bar cannot read shows its error and sends nothing.
fn run(state: AppState, tab: TabId) -> Out {
    if !state.tabs.owner.contains_key(&tab) {
        return (state, Vec::new());
    }
    let state = shown(state, tab);
    match submit_find(&state.find) {
        Ok(query) => gated(state, |s| super::guard_actions::run(s, query)),
        Err(find) => (AppState { find, ..state }, Vec::<Command>::new()),
    }
}

#[cfg(test)]
#[path = "tab_actions_tests.rs"]
mod tests;
