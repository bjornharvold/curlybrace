//! What the person does with tabs: open, duplicate, close, reopen, reorder, switch (#159).
//!
//! Every action ends with the tab it shows swapped into the foreground (`AppState::tab_current`),
//! so the code that reads `state.page` or `state.find` never learns that tabs exist. A tab opened
//! here runs one read, the default query of its collection; nothing else is sent unless a stale
//! tab is shown (it re-reads what it already read) or a dormant tab is told to Run.
use super::actions::gated;
use super::reducer::Out;
use super::state::{AppState, ConnStatus, Workspace};
use super::tab_navigation as nav;
use super::tab_seed::{Seed, seed_of, seeded};
use super::tab_shown::shown_now;
use super::tab_strip::TabNotice;
use super::tabs::TabId;
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
    /// The shortcuts act on the tab being looked at, which only the state knows.
    CloseActive,
    DuplicateActive,
    /// Move the tab being looked at this many places along the strip.
    MoveActive(i32),
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
        TabAction::Move { tab, to } => (nav::moved(state, tab, to), Vec::new()),
        TabAction::Next => nav::step(state, 1),
        TabAction::Previous => nav::step(state, -1),
        TabAction::NextMru => nav::mru(state, false),
        TabAction::PreviousMru => nav::mru(state, true),
        TabAction::Nth(at) => nav::nth(state, at),
        TabAction::Last => nav::last(state),
        TabAction::Run(tab) => run(state, tab),
        TabAction::CloseActive => match state.active_tab() {
            Some(tab) => close(state, tab),
            None => (state, Vec::new()),
        },
        TabAction::DuplicateActive => match state.active_tab() {
            Some(tab) => duplicate(state, tab),
            None => (state, Vec::new()),
        },
        TabAction::MoveActive(by) => (nav::moved_by(state, by), Vec::new()),
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
    // The tabs a person can see, plus none that are hidden: a session's only tab, when it is
    // empty and not shown, is not one they opened.
    state.visible_tabs().len() >= MAX_TABS
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
    let left = state.active_tab().filter(|left| *left != tab);
    let state = AppState {
        workspace: Workspace::Browse,
        ..leaving_view(state, session)
    };
    let shown = state.tab_current(tab);
    match left {
        Some(left) => without_empty(shown, left),
        None => shown,
    }
}

/// `left`, a tab nobody picked a collection in, is not worth keeping once it is left: only the
/// active empty tab is shown, so it would linger unseen. A session's only tab stays.
fn without_empty(state: AppState, left: TabId) -> AppState {
    let lonely = state
        .tabs
        .owner
        .get(&left)
        .is_none_or(|session| state.tabs.of_session(*session).len() < 2);
    // The same test as the sweep: a tab with a collection, a draft or an editor holds work.
    let idle = state.glance(left).is_some_and(|g| g.is_idle_empty());
    match lonely || !idle {
        true => state,
        // Nothing in it: nothing to remember either.
        false => state.tab_closed(left),
    }
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
    // A new tab reads its collection: not while the connection is down or being made.
    if state.status != ConnStatus::Connected {
        return (state, Vec::new());
    }
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
pub(super) fn activate(state: AppState, tab: TabId) -> Out {
    if !state.tabs.owner.contains_key(&tab) {
        return (state, Vec::new());
    }
    shown_now(shown(state, tab))
}

fn duplicate(state: AppState, tab: TabId) -> Out {
    let Some(session) = state.tabs.owner.get(&tab).copied() else {
        return (state, Vec::new());
    };
    if at_limit(&state) {
        return refused(state);
    }
    let (state, seed) = seed_of(state, tab);
    let Some(seed) = seed.map(Seed::without_editor) else {
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

/// Closes every tab of `keep`'s connection other than `keep` (or only those after it), then shows
/// `keep`. Another connection's tabs are not touched: they would be emptied and pushed onto the
/// closed stack by a click on this connection's tab. (The spec names the entries without a scope;
/// the strip shows every connection, so the menu acts on the tab's own.)
fn close_many(state: AppState, keep: TabId, right_only: bool) -> Out {
    let session = state.tabs.owner.get(&keep).copied();
    let visible: Vec<TabId> = state
        .visible_tabs()
        .into_iter()
        .filter(|t| state.tabs.owner.get(t).copied() == session)
        .collect();
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

/// A dormant tab's Run: the tab is shown and its find bar's query is sent, which is the one thing
/// that wakes it. A query the find bar cannot read shows its error and sends nothing.
fn run(state: AppState, tab: TabId) -> Out {
    if !state.tabs.owner.contains_key(&tab) {
        return (state, Vec::new());
    }
    let state = shown(state, tab);
    match submit_find(&state.find) {
        Ok(query) => gated(state, |s| {
            let indexes = s.tab == CollectionTab::Indexes;
            let (s, mut commands) = super::guard_actions::run(s, query);
            // On the Indexes sub-tab the list is what the tab shows: Run reads it too (the page
            // behind the Documents sub-tab is read as ever).
            if indexes && s.status == ConnStatus::Connected {
                let (s, more) = super::index_actions::switch_tab(s, CollectionTab::Indexes);
                commands.extend(more);
                return (s, commands);
            }
            (s, commands)
        }),
        Err(find) => (AppState { find, ..state }, Vec::<Command>::new()),
    }
}

#[cfg(test)]
#[path = "tab_activation_tests.rs"]
mod activation_tests;
#[cfg(test)]
#[path = "tab_dormant_tests.rs"]
mod dormant_tests;
#[cfg(test)]
#[path = "tab_hidden_tests.rs"]
mod hidden_tests;
#[cfg(test)]
#[path = "tab_review3_tests.rs"]
mod review3_tests;
#[cfg(test)]
#[path = "tab_review4_tests.rs"]
mod review4_tests;
#[cfg(test)]
#[path = "tab_review_fix_tests.rs"]
mod review_fix_tests;
#[cfg(test)]
#[path = "tab_actions_tests.rs"]
mod tests;
