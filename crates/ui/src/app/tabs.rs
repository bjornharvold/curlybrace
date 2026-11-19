//! The tab model: one collection view on one open session, and how it moves in and out of the
//! foreground (#159).
//!
//! The model is a second level of the swap `SessionView` already is. Every open session has one
//! *current tab*, whose fields live where the session's other browsing fields live: in `AppState`
//! while the session is active, inside its parked `SessionView` otherwise. The session's other
//! tabs are parked here in `AppState::parked_tabs`. `take_tab` and `restore_tab` swap the
//! tab-level fields of the foreground, so the hundreds of sites that read `state.page`,
//! `state.find` or `state.selected_ns()` never learn which tab they are in.
//!
//! The namespace of a tab is `sidebar.selected` while the tab is current: the swap moves it in and
//! out, so `selected_ns()` and the sidebar's "a selection in a vanished database is dropped" rule
//! keep working unchanged.
use super::explain_actions::VerdictState;
use super::guard_actions::GuardState;
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::aggregate_view::AggregateView;
use crate::viewmodel::{
    AggregateState, CollectionTab, EditorState, FindBarState, IndexesState, PageKey, TreeState,
    ViewMode,
};
use curlybrace_core::store::{Namespace, Page, Query};
use std::collections::BTreeMap;
use uuid::Uuid;

/// One tab. The number comes from a counter that never reuses one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabId(pub u64);

/// Whether a parked tab shows what the server holds. A later slice acts on it when the tab is
/// shown; until then every tab is `Fresh`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Freshness {
    #[default]
    Fresh,
    Stale,
    Dormant,
}

/// The tab-level fields: what one collection view holds. The current tab of each session keeps
/// them in the session's view; the others sit boxed in `AppState::parked_tabs`.
#[derive(Debug, Default)]
pub struct TabView {
    /// The collection shown; swapped with `sidebar.selected`.
    pub ns: Option<Namespace>,
    pub find: FindBarState,
    pub issued_query: Query,
    pub issued_request: u64,
    pub verdict_request: u64,
    pub page_query: Query,
    pub loading_page: bool,
    pub page: Option<Page>,
    pub page_key: Option<PageKey>,
    pub verdict: VerdictState,
    pub guard: Box<GuardState>,
    pub tree: TreeState,
    pub view_mode: ViewMode,
    pub tab: CollectionTab,
    pub indexes: IndexesState,
    pub aggregate: Box<AggregateState>,
    pub aggregate_view: Box<AggregateView>,
    pub editor: Option<EditorState>,
    pub freshness: Freshness,
}

impl AppState {
    /// Moves the foreground's tab fields out (leaving defaults); local mutation only, the public
    /// API stays immutable.
    #[inline(never)]
    fn take_tab(&mut self) -> TabView {
        TabView {
            ns: self.sidebar.selected.take(),
            find: std::mem::take(&mut self.find),
            issued_query: std::mem::take(&mut self.issued_query),
            issued_request: std::mem::take(&mut self.issued_request),
            verdict_request: std::mem::take(&mut self.verdict_request),
            page_query: std::mem::take(&mut self.page_query),
            loading_page: std::mem::take(&mut self.loading_page),
            page: self.page.take(),
            page_key: self.page_key.take(),
            verdict: std::mem::take(&mut self.verdict),
            guard: std::mem::take(&mut self.guard),
            tree: std::mem::take(&mut self.tree),
            view_mode: std::mem::take(&mut self.view_mode),
            tab: std::mem::take(&mut self.tab),
            indexes: std::mem::take(&mut self.indexes),
            aggregate: std::mem::take(&mut self.aggregate),
            aggregate_view: std::mem::take(&mut self.aggregate_view),
            editor: self.editor.take(),
            // The foreground tab is the one being looked at: fresh by definition.
            freshness: Freshness::Fresh,
        }
    }

    /// Puts `t` in the foreground. Destructures exhaustively: a field added to `TabView` but not
    /// restored here fails to compile.
    #[inline(never)]
    fn restore_tab(&mut self, t: TabView) {
        let TabView {
            ns,
            find,
            issued_query,
            issued_request,
            verdict_request,
            page_query,
            loading_page,
            page,
            page_key,
            verdict,
            guard,
            tree,
            view_mode,
            tab,
            indexes,
            aggregate,
            aggregate_view,
            editor,
            // Acted on when a tab is shown (a later slice); the foreground has no such field.
            freshness: _,
        } = t;
        self.sidebar.selected = ns;
        self.find = find;
        self.issued_query = issued_query;
        self.issued_request = issued_request;
        self.verdict_request = verdict_request;
        self.page_query = page_query;
        self.loading_page = loading_page;
        self.page = page;
        self.page_key = page_key;
        self.verdict = verdict;
        self.guard = guard;
        self.tree = tree;
        self.view_mode = view_mode;
        self.tab = tab;
        self.indexes = indexes;
        self.aggregate = aggregate;
        self.aggregate_view = aggregate_view;
        self.editor = editor;
    }

    /// A new, empty tab of `session`, parked until it is made current. The session's first tab
    /// is its current one instead: its fields are the session view's own. Unknown sessions get
    /// none.
    pub fn tab_opened(self, session: Uuid) -> (AppState, Option<TabId>) {
        if !self.open.contains(&session) {
            return (self, None);
        }
        let (tabs, tab) = self.tabs.clone().opened_for(session, None);
        let first = !self.tabs.current.contains_key(&session);
        let mut next = AppState { tabs, ..self };
        if !first {
            next.parked_tabs.insert(tab, Box::default());
        }
        (next, Some(tab))
    }

    /// `tab` closed. A parked tab just goes; the current tab is replaced by the session's first
    /// other tab, and a session's only tab stays (closing it would leave no view to show).
    pub fn tab_closed(self, tab: TabId) -> AppState {
        let Some(session) = self.tabs.owner.get(&tab).copied() else {
            return self;
        };
        if self.tabs.current.get(&session) == Some(&tab) {
            let Some(other) = self
                .tabs
                .of_session(session)
                .into_iter()
                .find(|t| *t != tab)
            else {
                return self;
            };
            return self.tab_made_current(session, other).tab_closed(tab);
        }
        let mut next = self;
        next.parked_tabs.remove(&tab);
        next.tabs = std::mem::take(&mut next.tabs).closed(tab);
        next
    }

    /// Every tab of `session` forgotten: the session is gone.
    pub(super) fn tabs_dropped_for(self, session: Uuid) -> AppState {
        let mut next = self;
        for tab in next.tabs.of_session(session) {
            next.parked_tabs.remove(&tab);
        }
        next.tabs = std::mem::take(&mut next.tabs).session_closed(session);
        next
    }

    /// The tabs of `session`, in strip order.
    pub fn tabs_of(&self, session: Uuid) -> Vec<TabId> {
        self.tabs.of_session(session)
    }

    /// The current tab of `session`, when it has one.
    pub fn current_tab_of(&self, session: Uuid) -> Option<TabId> {
        self.tabs.current.get(&session).copied()
    }

    /// Makes `tab` the current tab of its session: the session is made active (parking the
    /// previous one) and the tab swaps with the session's current one. A tab that is unknown or
    /// already current changes nothing but the session's activation.
    pub fn tab_current(self, tab: TabId) -> AppState {
        let Some(session) = self.tabs.owner.get(&tab).copied() else {
            return self;
        };
        let activated = self.activated(session);
        activated.tab_made_current(session, tab)
    }

    /// The swap itself, for a tab of `session`, which must be the active session.
    #[inline(never)]
    fn tab_made_current(self, session: Uuid, tab: TabId) -> AppState {
        let mut next = self;
        let Some(current) = next.tabs.current.get(&session).copied() else {
            return next;
        };
        if current == tab || !next.parked_tabs.contains_key(&tab) {
            return next;
        }
        let outgoing = next.take_tab();
        next.parked_tabs.insert(current, Box::new(outgoing));
        if let Some(incoming) = next.parked_tabs.remove(&tab) {
            next.restore_tab(*incoming);
        }
        next.tabs = std::mem::take(&mut next.tabs).made_current(session, tab);
        next
    }

    /// Runs `f` as if `tab` were the current tab of its (active) session, then puts both back.
    /// A tab that does not exist changes nothing.
    #[inline(never)]
    pub fn with_tab(self, tab: TabId, f: impl FnOnce(AppState) -> Out) -> Out {
        let Some(session) = self.tabs.owner.get(&tab).copied() else {
            return (self, Vec::new());
        };
        let Some(previous) = self.tabs.current.get(&session).copied() else {
            return (self, Vec::new());
        };
        self.with_session(session, |s| {
            let (s, commands) = f(s.tab_made_current(session, tab));
            // `f` may have closed the tab it ran in or its session; put back what still exists.
            let still_there = s.tabs.owner.get(&previous) == Some(&session) && s.is_open(session);
            match still_there && s.active == Some(session) {
                true => (s.tab_made_current(session, previous), commands),
                false => (s, commands),
            }
        })
    }

    /// Every parked tab of `session`, by id.
    pub fn parked_tabs_of(&self, session: Uuid) -> BTreeMap<TabId, &TabView> {
        self.tabs
            .of_session(session)
            .into_iter()
            .filter_map(|t| self.parked_tabs.get(&t).map(|v| (t, v.as_ref())))
            .collect()
    }
}

#[cfg(test)]
#[path = "tabs_tests.rs"]
mod tests;
