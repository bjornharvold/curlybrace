//! Reopening the saved tabs when their connection connects, without running anything (#159).
//!
//! At launch the saved tabs wait in `RestoreState::dormant`, grouped by connection id: nothing is
//! sent and nothing shows in the strip. When a connection connects, its saved tabs become
//! `Dormant` tabs holding their namespace, text, sub-tab and view mode. A dormant tab sends
//! nothing when shown; its Run is the only way its query goes out.
use super::persistence::Recovery;
use super::session::carried_aggregate;
use super::state::{AppState, Workspace};
use super::tab_actions::{MAX_TABS, activate};
use super::tab_strip::Split;
use super::tabs::TabId;
use super::workspace_app::WorkspaceSync;
use super::workspace_file::{
    MAX_SAVED_TABS, SavedDraftText, SavedFind, SavedSplit, SavedTab, WorkspaceFile,
};
use super::workspace_snapshot::saved_tab;
use crate::viewmodel::AggregateState;
use crate::viewmodel::aggregate_view::AggregateView;
use curlybrace_core::saved_pipeline::SavedPipeline;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// What the session restore keeps in the state.
#[derive(Debug)]
pub struct RestoreState {
    /// Whether the open tabs are saved and reopened (the "Reopen tabs at launch" switch).
    pub restore: bool,
    /// The saved tabs of connections that have not connected yet.
    pub dormant: BTreeMap<Uuid, Vec<SavedTab>>,
    /// The saved active tab when its connection has not connected yet: the connection and the
    /// position among that connection's saved tabs.
    pub active_hint: Option<(Uuid, usize)>,
    /// The saved split when both its panes belong to one connection: that connection, and the
    /// split with its secondary as a position among the connection's saved tabs.
    pub split_hint: Option<(Uuid, SavedSplit)>,
    /// Where and when the file is written.
    pub sync: WorkspaceSync,
}

impl Default for RestoreState {
    fn default() -> Self {
        Self {
            restore: true,
            dormant: BTreeMap::new(),
            active_hint: None,
            split_hint: None,
            sync: WorkspaceSync::default(),
        }
    }
}

impl RestoreState {
    /// The saved tabs of `file`, grouped by connection; nothing when the person turned
    /// restoring off.
    pub(super) fn of_file(file: &WorkspaceFile, sync: WorkspaceSync) -> Self {
        let mut dormant: BTreeMap<Uuid, Vec<SavedTab>> = BTreeMap::new();
        let mut place: Vec<(Uuid, usize)> = Vec::new();
        for tab in &file.tabs {
            let group = dormant.entry(tab.connection).or_default();
            place.push((tab.connection, group.len()));
            group.push(tab.clone());
        }
        let active_hint = file.active.and_then(|a| place.get(a).copied());
        let split_hint = file.split.and_then(|split| {
            let (connection, secondary) = place.get(split.secondary).copied()?;
            let (active, at) = active_hint?;
            // A split of a tab with itself is no split.
            (active == connection && secondary != at)
                .then_some((connection, SavedSplit { secondary, ..split }))
        });
        Self {
            restore: file.restore,
            dormant,
            active_hint,
            split_hint,
            sync,
        }
    }

    /// Only the saved tabs of connections in `known` stay.
    pub(super) fn known_only(&mut self, known: &BTreeSet<Uuid>) {
        self.dormant.retain(|id, _| known.contains(id));
        let kept = |id: &Uuid| self.dormant.contains_key(id);
        if self.active_hint.as_ref().is_some_and(|(id, _)| !kept(id)) {
            self.active_hint = None;
        }
        if self.split_hint.as_ref().is_some_and(|(id, _)| !kept(id)) {
            self.split_hint = None;
        }
    }
}

impl AppState {
    /// The saved tabs of the connection `id` dropped (it was deleted). Those of connections the
    /// list failed to load are not touched: they are unknown, not deleted.
    pub(super) fn dormant_forgotten(mut self, id: Uuid) -> AppState {
        let known: BTreeSet<Uuid> = self
            .restore
            .dormant
            .keys()
            .copied()
            .filter(|k| *k != id)
            .collect();
        self.restore.known_only(&known);
        self
    }
}

/// `session` has connected: its saved tabs become dormant tabs, in the strip's order after the
/// tabs already open, and the saved active tab is shown when the person is looking at this
/// connection. Sends nothing. The fast path (nothing saved for `session`) does no work, because
/// it is on the deepest event path.
pub(super) fn materialise(state: &mut AppState, session: Uuid, background: bool) {
    if state.restore.dormant.contains_key(&session) {
        reopened(state, session, background, true);
    }
}

/// Room appeared (a tab was closed): the saved tabs of connected sessions that did not fit reopen,
/// oldest first, dormant and sending nothing, behind whatever the person is looking at.
pub(super) fn refill(state: &mut AppState) {
    if state.restore.dormant.is_empty() {
        return;
    }
    let waiting: Vec<Uuid> = state
        .open
        .iter()
        .copied()
        .filter(|s| state.restore.dormant.contains_key(s))
        .filter(|s| state.status_of(*s) == Some(super::state::ConnStatus::Connected))
        .collect();
    for session in waiting {
        if room_for(state, false, None) == 0 {
            return;
        }
        reopened(state, session, true, false);
    }
}

#[inline(never)]
fn reopened(state: &mut AppState, session: Uuid, background: bool, initial: bool) {
    let mut saved = state.restore.dormant.remove(&session).unwrap_or_default();
    let active = state
        .restore
        .active_hint
        .filter(|(id, _)| *id == session)
        .map(|(_, at)| at);
    let split = state
        .restore
        .split_hint
        .take_if(|(id, _)| *id == session)
        .map(|(_, split)| split);
    // Only the connect itself takes the front; a tab reopened later waits behind it.
    let front = initial && in_front(state, session, background);
    let room = room_for(state, front, active);
    // What does not fit keeps waiting, in the file too: nothing saved is dropped for lack of room.
    let rest = saved.split_off(room.min(saved.len()));
    let opened_count = saved.len();
    if active.is_some() {
        // The saved active tab keeps its place among the tabs that still wait.
        state.restore.active_hint = active
            .filter(|at| *at >= opened_count)
            .map(|at| (session, at - opened_count));
    }
    let overflow = rest.len();
    if !rest.is_empty() {
        state.restore.dormant.insert(session, rest);
    }
    let mut next = std::mem::take(state);
    let mut opened: Vec<Option<TabId>> = Vec::new();
    for tab in &saved {
        let after = opened.iter().rev().flatten().next().copied();
        let (n, id) = tab_opened(next, session, after, tab);
        next = n;
        opened.push(id);
    }
    let at = |i: usize| opened.get(i).copied().flatten();
    if let Some(tab) = active.and_then(at).filter(|_| front) {
        next = shown(next, tab);
        next = paired(next, tab, split, at);
    }
    if initial && overflow > 0 {
        let notice = Recovery::WorkspaceWaiting { count: overflow };
        next = next.with_recovery_notice(notice);
    }
    *state = next;
}

/// How many tabs may still be opened: the limit counts every tab the strip shows, an empty one in
/// front included, unless the saved active tab is about to take its place.
fn room_for(state: &AppState, front: bool, active: Option<usize>) -> usize {
    let held = held(state);
    let shown = state.visible_tabs().len();
    let replaced =
        front && shown > held && active.is_some_and(|i| i < MAX_TABS.saturating_sub(held));
    MAX_TABS.saturating_sub(if replaced { held } else { shown })
}

/// The tabs that hold something: those that count against the limit.
fn held(state: &AppState) -> usize {
    state
        .visible_tabs()
        .into_iter()
        .filter(|tab| state.glance(*tab).is_some_and(|g| !g.is_idle_empty()))
        .count()
}

/// Whether the person is looking at `session` in the Browse workspace (a connection that
/// connects behind another one takes nothing from it).
fn in_front(state: &AppState, session: Uuid, background: bool) -> bool {
    !background && state.active == Some(session) && state.workspace == Workspace::Browse
}

/// A new dormant tab of `session` holding `saved`'s namespace, text, sub-tab and view mode.
fn tab_opened(
    state: AppState,
    session: Uuid,
    after: Option<TabId>,
    saved: &SavedTab,
) -> (AppState, Option<TabId>) {
    let (mut state, tab) = state.tab_opened_after(session, after);
    let Some(tab) = tab else {
        return (state, None);
    };
    let Some(view) = state.parked_tabs.get_mut(&tab) else {
        return (state, None);
    };
    let (aggregate, aggregate_view) = restored_draft(saved.draft.as_ref());
    view.ns = Some(saved.namespace());
    view.find = saved
        .find
        .clone()
        .map(SavedFind::into_bar)
        .unwrap_or_default();
    view.tab = saved.sub_tab.into();
    view.view_mode = saved.view_mode.into();
    view.aggregate = aggregate;
    view.aggregate_view = aggregate_view;
    state.tabs = std::mem::take(&mut state.tabs).made_restored(tab, saved.text_dropped);
    (state, Some(tab))
}

/// The draft of a restored tab: its stages as typed, no preview, and no preview asked for until
/// the person edits or refreshes (a dormant tab sends nothing on its own).
fn restored_draft(draft: Option<&SavedDraftText>) -> (Box<AggregateState>, Box<AggregateView>) {
    let base = AggregateState::default();
    let state = match draft {
        Some(text) => {
            let draft = base
                .opened_from(&SavedPipeline {
                    id: Uuid::nil(),
                    name: String::new(),
                    ns_hint: None,
                    saved_unix: 0,
                    stages: text.stages.clone(),
                    options: text.options.clone(),
                })
                .without_opened();
            match text.text_mode {
                true => draft.to_text(),
                false => draft,
            }
        }
        None => base,
    };
    carried_aggregate(&state, &AggregateView::default())
}

/// `tab` shown. Through the one step every shown tab takes; a dormant tab sends nothing there.
fn shown(state: AppState, tab: TabId) -> AppState {
    let (state, commands) = activate(state, tab);
    debug_assert!(
        commands.is_empty(),
        "a dormant tab sends nothing: {commands:?}"
    );
    state
}

/// The saved split, when its other pane was reopened too: `active` (shown now) and that pane.
fn paired(
    state: AppState,
    active: TabId,
    split: Option<SavedSplit>,
    at: impl Fn(usize) -> Option<TabId>,
) -> AppState {
    let Some((split, partner)) = split
        .and_then(|s| Some((s, at(s.secondary)?)))
        .filter(|(_, partner)| *partner != active)
    else {
        return state;
    };
    let split = Split {
        primary: active,
        secondary: partner,
        ratio: crate::viewmodel::split_layout::clamped(split.ratio),
        orientation: split.orientation.into(),
        secondary_first: split.secondary_first,
    };
    AppState {
        tabs: state.tabs.clone().with_split(Some(split)),
        ..state
    }
}

impl AppState {
    /// `session`'s open tabs go back to waiting for its next connect: they stay in the saved file
    /// and reopen, dormant, when it connects again. For a connection that is closed (but still
    /// saved) and for a runtime that stopped. Tabs already waiting for room come after them.
    pub(super) fn tabs_waiting(mut self, session: Uuid) -> AppState {
        if !self.restore.restore || self.connection(session).is_none() {
            return self;
        }
        let open: Vec<(TabId, SavedTab)> = self
            .tabs
            .of_session(session)
            .into_iter()
            .filter_map(|tab| Some((tab, saved_tab(&self, tab)?)))
            .collect();
        if open.is_empty() {
            return self;
        }
        let place = |tab: TabId| open.iter().position(|(t, _)| *t == tab);
        let front = self.active_tab().and_then(place);
        let split = self.tabs.split.clone().and_then(|s| {
            let (primary, secondary) = (place(s.primary)?, place(s.secondary)?);
            (Some(primary) == front && primary != secondary).then_some((secondary, s))
        });
        // The tabs that were already waiting stay first: they have waited longest, and reopening
        // takes them in order, so tabs left open are never ahead of them.
        let waiting = self.restore.dormant.remove(&session).unwrap_or_default();
        let before = waiting.len();
        let mut tabs = waiting;
        tabs.extend(open.into_iter().map(|(_, saved)| saved));
        tabs.truncate(MAX_SAVED_TABS);
        self.restore.dormant.insert(session, tabs);
        if let Some(at) = front {
            self.restore.active_hint = Some((session, before + at));
            self.restore.split_hint = split.map(|(secondary, s)| {
                let saved = SavedSplit {
                    secondary: before + secondary,
                    ratio: s.ratio,
                    orientation: s.orientation.into(),
                    secondary_first: s.secondary_first,
                };
                (session, saved)
            });
        }
        self
    }

    /// Every open session's tabs back to waiting (the sessions are all gone).
    pub(super) fn all_tabs_waiting(self) -> AppState {
        let open = self.open.clone();
        open.into_iter().fold(self, AppState::tabs_waiting)
    }
}

#[cfg(test)]
#[path = "workspace_restore_tests.rs"]
mod tests;
