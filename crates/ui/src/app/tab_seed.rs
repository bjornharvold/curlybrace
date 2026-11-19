//! What a duplicated or reopened tab starts from: the text and choices of another tab, never its
//! results (#159).
use super::actions::issue_find;
use super::reducer::Out;
use super::session::carried_aggregate;
use super::state::{AppState, ConnStatus};
use super::tab_strip::ClosedTab;
use super::tabs::TabId;
use crate::viewmodel::aggregate_run::RunUi;
use crate::viewmodel::{AggregateState, CollectionTab, EditorState, FindBarState, ViewMode};
use curlybrace_core::store::{Namespace, Query};
use uuid::Uuid;

pub(super) struct Seed {
    ns: Option<Namespace>,
    find: FindBarState,
    sub: CollectionTab,
    view_mode: ViewMode,
    aggregate: Box<AggregateState>,
    editor: Option<EditorState>,
    query: Query,
    dormant: bool,
}

impl Seed {
    /// The foreground tab's text and choices: never its results, nothing in flight.
    fn of_foreground(state: &AppState, dormant: bool) -> Self {
        let (aggregate, _) = carried_aggregate(&state.aggregate, &state.aggregate_view);
        // A copy ran nothing: it takes none of the source's run (its number would route the
        // source's answer to the copy, and "outcome unknown" would be a lie, the connection never
        // dropped). `carried_aggregate` already cleared the previews, explain, counts and hint list.
        let aggregate = Box::new(aggregate.with_run(RunUi::default()));
        Self {
            ns: state.selected_ns().cloned(),
            find: state.find.clone(),
            sub: state.tab,
            view_mode: state.view_mode,
            aggregate,
            editor: state.editor.clone(),
            query: state.page_query.clone(),
            dormant,
        }
    }

    pub(super) fn has_collection(&self) -> bool {
        self.ns.is_some()
    }

    pub(super) fn closed(self, session: Uuid) -> ClosedTab {
        ClosedTab {
            session,
            ns: self.ns,
            find: self.find,
            tab: self.sub,
            view_mode: self.view_mode,
            aggregate: self.aggregate,
            editor: self.editor,
            dormant: self.dormant,
            query: self.query,
        }
    }

    pub(super) fn of_closed(closed: ClosedTab) -> Self {
        Self {
            ns: closed.ns,
            find: closed.find,
            sub: closed.tab,
            view_mode: closed.view_mode,
            aggregate: closed.aggregate,
            editor: closed.editor,
            query: closed.query,
            dormant: closed.dormant,
        }
    }
}

/// The seed of `tab`, read where its fields live.
pub(super) fn seed_of(state: AppState, tab: TabId) -> (AppState, Option<Seed>) {
    let dormant = state.tabs.dormant.contains(&tab);
    let mut seed = None;
    let (state, _) = state.with_tab(tab, |s| {
        seed = Some(Seed::of_foreground(&s, dormant));
        (s, Vec::new())
    });
    (state, seed)
}

/// Fills the (new, empty, current) foreground tab from `seed` and, unless it is dormant or
/// empty, reads its page with the seed's query: one read the source already ran.
pub(super) fn seeded(state: AppState, seed: Seed, tab: TabId) -> Out {
    let Seed {
        ns,
        find,
        sub,
        view_mode,
        aggregate,
        editor,
        query,
        dormant,
    } = seed;
    let Some(ns) = ns else {
        let next = AppState {
            find,
            tab: sub,
            view_mode,
            aggregate,
            ..state
        };
        return (next, Vec::new());
    };
    let state = AppState {
        sidebar: state.sidebar.selecting(ns),
        find,
        tab: sub,
        view_mode,
        aggregate,
        editor,
        ..state
    };
    if dormant || state.status != ConnStatus::Connected {
        let tabs = state.tabs.clone().made_dormant([tab]);
        return (AppState { tabs, ..state }, Vec::new());
    }
    let (state, mut commands) = issue_find(state, query);
    let (state, more) = super::index_actions::collection_opened(state);
    commands.extend(more);
    (state, commands)
}
