//! The sites that act on a whole session reach every tab of it (#159).
//!
//! The current tab of a session is handled by the code that always handled "the session's view";
//! these helpers carry the same decision to the session's *parked* tabs: a lost connection, a
//! write answered, a database that vanished or a collection dropped, a status the sampler or the
//! profiler reports. Each applies to the parked tabs of one session and never to another's.
use super::explain_actions::VerdictState;
use super::guard_actions::GuardState;
use super::session::carried_aggregate;
use super::state::AppState;
use super::tabs::{Freshness, TabView};
use crate::viewmodel::{IndexesState, ProfilingView};
use curlybrace_core::runtime::{ProfilerStatus, SamplerStatus};
use curlybrace_core::store::{Namespace, ProfilingState};
use uuid::Uuid;

/// Applies `f` to every parked tab of `session`. In place, with local mutation only: the callers
/// are on the deepest event paths, where a by-value `AppState` would cost a frame each.
#[inline(never)]
pub(super) fn map_parked_tabs(state: &mut AppState, session: Uuid, f: impl Fn(TabView) -> TabView) {
    for tab in state.tabs.of_session(session) {
        if let Some(view) = state.parked_tabs.remove(&tab) {
            state.parked_tabs.insert(tab, Box::new(f(*view)));
        }
    }
}

impl TabView {
    /// What a lost connection leaves of a tab: nothing it showed, but the Aggregate draft, which
    /// is the person's work (its preview made stale, as for the session's current tab).
    pub fn on_connection_lost(self) -> TabView {
        let (aggregate, aggregate_view) = carried_aggregate(&self.aggregate, &self.aggregate_view);
        TabView {
            aggregate,
            aggregate_view,
            freshness: self.freshness,
            ..TabView::default()
        }
    }

    /// The tab's page predates a write to `ns`: it is stale when it shows that collection.
    pub fn marked_stale_if_on(self, ns: &Namespace) -> TabView {
        match self.ns.as_ref() == Some(ns) {
            true => TabView {
                freshness: Freshness::Stale,
                ..self
            },
            false => self,
        }
    }

    /// A database list without the tab's database drops its collection, as the session's
    /// current tab loses its selection.
    pub fn without_vanished(self, databases: &[String]) -> TabView {
        let listed = self.ns.as_ref().is_none_or(|ns| databases.contains(&ns.db));
        match listed {
            true => self,
            false => TabView { ns: None, ..self },
        }
    }

    /// The sampler of the session reported `status` (it belongs to the session, not to a tab).
    pub fn with_sampler(self, status: SamplerStatus) -> TabView {
        TabView {
            indexes: self.indexes.with_sampler(status),
            ..self
        }
    }

    /// The profiler of the session reported `status`.
    pub fn with_profiler(self, status: ProfilerStatus) -> TabView {
        let queries = self.indexes.queries.clone().with_status(status);
        TabView {
            indexes: self.indexes.with_queries(queries),
            ..self
        }
    }

    /// The collection the tab showed is gone: nothing is open, as for the current tab (see
    /// `admin_dialog_actions::deselected`). The draft stays, its preview made idle.
    pub fn deselected(self) -> TabView {
        TabView {
            ns: None,
            find: Default::default(),
            loading_page: false,
            page: None,
            page_key: None,
            verdict: VerdictState::default(),
            guard: Box::<GuardState>::default(),
            tree: self.tree.results_replaced(),
            editor: None,
            indexes: IndexesState::default(),
            aggregate: Box::new(self.aggregate.on_collection_changed()),
            ..self
        }
    }

    /// The database's profiling level is `level`: shown in the tab's Queries view when the tab
    /// is on that database.
    pub fn with_profiling_level(self, db: &str, level: ProfilingState) -> TabView {
        match self.indexes.ns.as_ref().is_some_and(|ns| ns.db == db) {
            true => {
                let queries = self
                    .indexes
                    .queries
                    .clone()
                    .with_profiling(ProfilingView::Known(level));
                TabView {
                    indexes: self.indexes.with_queries(queries),
                    ..self
                }
            }
            false => self,
        }
    }
}

/// The connection of `session` was lost: every parked tab is reset. Nothing to do (and no frame
/// of any size) for the usual session with one tab: this is called at the tip of the deepest event
/// path.
pub(super) fn connection_lost(state: &mut AppState, session: Uuid) {
    if !state.parked_tabs.is_empty() {
        map_parked_tabs(state, session, TabView::on_connection_lost);
    }
}

/// Whether a parked tab of the active session has an editor or a modal open: one blip must not
/// discard what is typed in any tab. False at once when nothing is parked (the usual case, on the
/// deepest event path).
pub(super) fn parked_tab_typing(state: &AppState) -> bool {
    if state.parked_tabs.is_empty() {
        return false;
    }
    let session = state.session_id();
    state.tabs.of_session(session).iter().any(|tab| {
        state
            .parked_tabs
            .get(tab)
            .is_some_and(|view| view.editor.is_some() || view.indexes.has_prompt())
    })
}

/// The database list changed: the sidebar takes it, and parked tabs whose database vanished lose
/// their collection (the current tab loses its selection with the sidebar's own rule).
#[inline(never)]
pub(super) fn databases_listed(state: AppState, names: Vec<String>) -> AppState {
    let sidebar = state.sidebar.with_databases(names);
    let mut state = AppState { sidebar, ..state };
    let session = state.session_id();
    let names: Vec<String> = state
        .sidebar
        .databases
        .iter()
        .map(|d| d.name.clone())
        .collect();
    map_parked_tabs(&mut state, session, |tab| tab.without_vanished(&names));
    state
}

/// The sampler reported: every tab of the session shows it.
#[inline(never)]
pub(super) fn sampler_reported(mut state: AppState, status: SamplerStatus) -> AppState {
    let session = state.session_id();
    map_parked_tabs(&mut state, session, |tab| tab.with_sampler(status));
    super::index_actions::sampler_reported(state, status)
}

/// The profiler reported: every tab of the session shows it.
#[inline(never)]
pub(super) fn profiler_reported(mut state: AppState, status: ProfilerStatus) -> AppState {
    let session = state.session_id();
    map_parked_tabs(&mut state, session, |tab| tab.with_profiler(status.clone()));
    super::queries_actions::profiler_reported(state, status)
}

/// A write to `ns` was answered. Closes the editor of the tab it was sent from when that is a
/// parked tab (and the editor still holds what was saved), and marks every parked tab on `ns`
/// stale. Returns whether the session's *current* tab's editor may close for this answer: only
/// when the write came from it (or from no tab), never when it came from another or a closed one.
pub(super) fn written(mut state: AppState, ns: &Namespace) -> (AppState, bool) {
    let session = state.session_id();
    let pending = state.pending.clone().filter(|p| p.ns == *ns);
    let writer = pending.as_ref().and_then(|p| p.tab);
    let current = state.current_tab_of(session);
    let current_may_close = writer.is_none() || writer == current;
    if let (Some(tab), Some(key)) = (writer, pending.and_then(|p| p.editor)) {
        if let Some(view) = state.parked_tabs.get_mut(&tab) {
            let holds = view.editor.as_ref().is_some_and(|e| key.held_by(e));
            if holds {
                view.editor = None;
            }
        }
    }
    map_parked_tabs(&mut state, session, |tab| tab.marked_stale_if_on(ns));
    (state, current_may_close)
}

/// An index write on `ns` succeeded. Settles it in the parked tab it was sent from (the current
/// tab's own write is settled by the caller as before) and marks every parked tab on `ns` stale.
pub(super) fn index_written(state: AppState, ns: &Namespace) -> AppState {
    let session = state.session_id();
    let writer = state
        .pending
        .as_ref()
        .filter(|p| p.ns == *ns)
        .and_then(|p| p.tab)
        .filter(|tab| state.parked_tabs.contains_key(tab));
    let mut state = match writer {
        Some(tab) => {
            state
                .with_tab(tab, |s| {
                    (
                        super::index_write_actions::write_finished(s, ns),
                        Vec::new(),
                    )
                })
                .0
        }
        None => state,
    };
    map_parked_tabs(&mut state, session, |tab| tab.marked_stale_if_on(ns));
    state
}

/// An administration write took away what `hides` says: every parked tab showing it is blanked.
pub(super) fn dropped(mut state: AppState, hides: impl Fn(&Namespace) -> bool) -> AppState {
    let session = state.session_id();
    map_parked_tabs(&mut state, session, |tab| {
        match tab.ns.as_ref().is_some_and(&hides) {
            true => tab.deselected(),
            false => tab,
        }
    });
    state
}

/// The profiling level of `db` was read or set: every parked tab on that database shows it.
pub(super) fn profiling_level(mut state: AppState, db: &str, level: &ProfilingState) -> AppState {
    let session = state.session_id();
    map_parked_tabs(&mut state, session, |tab| {
        tab.with_profiling_level(db, *level)
    });
    state
}

#[cfg(test)]
#[path = "tab_fanout_guard_tests.rs"]
mod guard_tests;
#[cfg(test)]
#[path = "tab_profiler_tests.rs"]
mod profiler_tests;
#[cfg(test)]
#[path = "tab_review_tests.rs"]
mod review_tests;
#[cfg(test)]
#[path = "tab_fanout_tests.rs"]
mod tests;
