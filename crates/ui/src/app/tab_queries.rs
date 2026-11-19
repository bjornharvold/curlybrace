//! What the app can say about any tab without swapping it in: its collection, its sub-tab, whether
//! it holds unsaved work, which tabs the strip shows (#159). Read-only: the current tab of the
//! active session is read from the foreground fields, the current tab of another session from its
//! parked view, and every other tab from `parked_tabs`.
use super::state::AppState;
use super::tabs::{Freshness, TabId, TabView};
use crate::viewmodel::{CollectionTab, ViewMode};
use curlybrace_core::store::Namespace;

/// The few things about a tab that the strip, the menu and the shortcuts need.
#[derive(Clone, Debug, PartialEq)]
pub struct TabGlance {
    pub ns: Option<Namespace>,
    pub sub: CollectionTab,
    pub view_mode: ViewMode,
    /// An editor or a modal of the index writes is open: closing the tab or losing the server
    /// would discard what was typed.
    pub unsaved: bool,
}

impl TabGlance {
    fn of_parked(view: &TabView) -> Self {
        Self {
            ns: view.ns.clone(),
            sub: view.tab,
            view_mode: view.view_mode,
            unsaved: view.editor.is_some() || view.indexes.has_prompt(),
        }
    }
}

impl AppState {
    /// The tab the person is looking at: the current tab of the active session.
    pub fn active_tab(&self) -> Option<TabId> {
        self.active
            .and_then(|session| self.tabs.current.get(&session).copied())
    }

    /// What `tab` shows, wherever its fields live.
    pub fn glance(&self, tab: TabId) -> Option<TabGlance> {
        let session = self.tabs.owner.get(&tab).copied()?;
        if self.tabs.current.get(&session) != Some(&tab) {
            return self.parked_tabs.get(&tab).map(|v| TabGlance::of_parked(v));
        }
        let (ns, sub, view_mode, unsaved) = match self.active == Some(session) {
            true => (
                self.sidebar.selected.clone(),
                self.tab,
                self.view_mode,
                self.editor.is_some() || self.indexes.has_prompt(),
            ),
            false => {
                let v = self.parked.get(&session)?;
                (
                    v.sidebar.selected.clone(),
                    v.tab,
                    v.view_mode,
                    v.editor.is_some() || v.indexes.has_prompt(),
                )
            }
        };
        Some(TabGlance {
            ns,
            sub,
            view_mode,
            unsaved,
        })
    }

    /// The collection `tab` shows.
    pub fn tab_ns(&self, tab: TabId) -> Option<Namespace> {
        self.glance(tab).and_then(|g| g.ns)
    }

    /// The tabs the strip shows, in strip order: those with a collection, and the active tab even
    /// when it is empty (only the active empty tab is shown).
    pub fn visible_tabs(&self) -> Vec<TabId> {
        let active = self.active_tab();
        self.tabs
            .order
            .iter()
            .copied()
            .filter(|t| Some(*t) == active || self.tab_ns(*t).is_some())
            .collect()
    }

    /// Whether `tab` shows what the server holds, is known not to, or has not run since the
    /// connection was lost. The current tab is never stale: it is being looked at (or, in another
    /// session, kept fresh by that session's own answers).
    pub fn freshness_of(&self, tab: TabId) -> Freshness {
        if self.tabs.dormant.contains(&tab) {
            return Freshness::Dormant;
        }
        match self.parked_tabs.get(&tab) {
            Some(view) => view.freshness,
            None if self.tabs.stale.contains(&tab) => Freshness::Stale,
            None => Freshness::Fresh,
        }
    }

    /// The tab of the strip's visible list that sits `step` places from `from` (wrapping), or the
    /// first one when `from` is not shown.
    pub fn visible_neighbour(&self, from: Option<TabId>, step: isize) -> Option<TabId> {
        let visible = self.visible_tabs();
        let at = from.and_then(|t| visible.iter().position(|v| *v == t));
        let count = isize::try_from(visible.len()).ok()?;
        match at.and_then(|p| isize::try_from(p).ok()) {
            Some(p) if count > 0 => {
                let next = (p + step).rem_euclid(count);
                visible.get(usize::try_from(next).ok()?).copied()
            }
            _ => visible.first().copied(),
        }
    }
}
