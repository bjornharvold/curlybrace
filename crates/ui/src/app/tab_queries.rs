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
    /// An Aggregate draft with a stage: the person's work, which a blank tab still holds.
    pub has_draft: bool,
}

impl TabGlance {
    fn of_parked(view: &TabView) -> Self {
        Self {
            ns: view.ns.clone(),
            sub: view.tab,
            view_mode: view.view_mode,
            unsaved: view.editor.is_some() || view.indexes.has_prompt(),
            has_draft: !view.aggregate.stages.is_empty(),
        }
    }

    /// No collection and no work: nothing to show and nothing to lose.
    pub fn is_idle_empty(&self) -> bool {
        self.ns.is_none() && !self.unsaved && !self.has_draft
    }
}

impl AppState {
    /// The tab the person is looking at: the current tab of the active session.
    pub fn active_tab(&self) -> Option<TabId> {
        self.active
            .and_then(|session| self.tabs.current.get(&session).copied())
    }

    /// Whether any dialog of the state is open: the one list the tab shortcuts read
    /// (`tab_strip_panel::shortcuts_enabled`), so a dialog is named in one place. A dialog that
    /// lives outside the state, the SSH host-key question, is caught there by egui's modal layer
    /// instead.
    pub fn any_dialog_open(&self) -> bool {
        self.editor.is_some()
            || self.indexes.has_prompt()
            || self.write_warning.is_some()
            || self.admin.is_some()
            || self.bulk.is_some()
            || self.copy.is_some()
            || self.copies.is_some()
            || self.forget_confirm.is_some()
            || self.tools.dialog().is_some()
            || self.query_history.ui.dialog.is_some()
            || self.code_export.dialog.is_some()
            || self.aggregate.run.dialog.is_some()
            || self.aggregate.saved.dialog.is_some()
            || self.tree.confirm != crate::viewmodel::doc_actions::DeleteConfirm::None
    }

    /// The foreground tab sent (or is about to send) a find: it is no longer waiting for Run after
    /// a lost connection.
    pub(super) fn awake_current(self) -> AppState {
        match self.active.and_then(|s| self.tabs.current.get(&s).copied()) {
            // Its read is on its way: whatever made it dormant or stale is answered by it.
            Some(tab) if self.tabs.dormant.contains(&tab) || self.tabs.stale.contains(&tab) => {
                let tabs = self.tabs.clone().awakened(tab).with_stale(tab, false);
                // Awake, the Aggregate tab previews on its next show as a fresh tab does.
                let aggregate_view = Box::new(crate::viewmodel::aggregate_view::AggregateView {
                    auto_sent: None,
                    ..(*self.aggregate_view).clone()
                });
                AppState {
                    tabs,
                    aggregate_view,
                    ..self
                }
            }
            _ => self,
        }
    }

    /// The foreground tab is known to show less than the server holds.
    pub(super) fn current_marked_stale(self) -> AppState {
        match self.active.and_then(|s| self.tabs.current.get(&s).copied()) {
            Some(tab) => {
                let tabs = self.tabs.clone().with_stale(tab, true);
                AppState { tabs, ..self }
            }
            None => self,
        }
    }

    /// The tab being looked at lost its connection and has not run since.
    pub fn dormant_active(&self) -> bool {
        self.active_tab()
            .is_some_and(|tab| self.tabs.dormant.contains(&tab))
    }

    /// The tab being looked at was reopened from the saved session and has not run yet.
    pub fn active_tab_restored(&self) -> bool {
        self.active_tab()
            .is_some_and(|tab| self.tabs.restored.contains(&tab))
    }

    /// The tab being looked at was reopened without its query text (it was too long to save).
    pub fn active_tab_text_dropped(&self) -> bool {
        self.active_tab()
            .is_some_and(|tab| self.tabs.text_dropped.contains(&tab))
    }

    /// The tab being looked at was reopened with its saved query options dropped (not valid).
    pub fn active_tab_options_dropped(&self) -> bool {
        self.active_tab()
            .is_some_and(|tab| self.tabs.options_dropped.contains(&tab))
    }

    /// What `tab` shows, wherever its fields live.
    pub fn glance(&self, tab: TabId) -> Option<TabGlance> {
        let session = self.tabs.owner.get(&tab).copied()?;
        if self.tabs.current.get(&session) != Some(&tab) {
            return self.parked_tabs.get(&tab).map(|v| TabGlance::of_parked(v));
        }
        let (ns, sub, view_mode, unsaved, has_draft) = match self.active == Some(session) {
            true => (
                self.sidebar.selected.clone(),
                self.tab,
                self.view_mode,
                self.editor.is_some() || self.indexes.has_prompt(),
                !self.aggregate.stages.is_empty(),
            ),
            false => {
                let v = self.parked.get(&session)?;
                (
                    v.sidebar.selected.clone(),
                    v.tab,
                    v.view_mode,
                    v.editor.is_some() || v.indexes.has_prompt(),
                    !v.aggregate.stages.is_empty(),
                )
            }
        };
        Some(TabGlance {
            ns,
            sub,
            view_mode,
            unsaved,
            has_draft,
        })
    }

    /// The collection `tab` shows.
    pub fn tab_ns(&self, tab: TabId) -> Option<Namespace> {
        self.glance(tab).and_then(|g| g.ns)
    }

    /// The tabs the strip shows, in strip order: those with a collection or with work in them (an
    /// editor, a dialog, an Aggregate draft), and the active tab and the split's partner even when
    /// they are empty (both are on screen). An empty tab that is not shown is swept
    /// (`tab_eviction::sweep_empty`, which agrees), except a session's only one.
    pub fn visible_tabs(&self) -> Vec<TabId> {
        let active = self.active_tab();
        let partner = self.tabs.split.as_ref().map(|s| s.secondary);
        self.tabs
            .order
            .iter()
            .copied()
            .filter(|t| {
                Some(*t) == active
                    || Some(*t) == partner
                    || self.glance(*t).is_some_and(|g| !g.is_idle_empty())
            })
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
