//! Reopening the saved tabs when their connection connects, without running anything (#159).
//!
//! At launch the saved tabs wait in `RestoreState::dormant`, grouped by connection id: nothing is
//! sent and nothing shows in the strip. When a connection connects, its saved tabs become
//! `Dormant` tabs holding their namespace, text, sub-tab and view mode. A dormant tab sends
//! nothing when shown; its Run is the only way its query goes out.
use super::persistence::Recovery;
use super::session::carried_aggregate;
use super::state::{AppState, Banner, Workspace};
use super::tab_actions::{MAX_TABS, activate};
use super::tab_strip::Split;
use super::tabs::TabId;
use super::workspace_app::WorkspaceSync;
use super::workspace_file::{
    MAX_SAVED_TABS, SavedDraftText, SavedFind, SavedSplit, SavedTab, WorkspaceFile,
};
use super::workspace_snapshot::{is_saved_tab, saved_tab};
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
    /// The saved front tab that was reopened but not shown (the person was somewhere else): it
    /// stays the front tab of the next write until another saved tab is in front.
    pub front_live: Option<TabId>,
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
            front_live: None,
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
            front_live: None,
            sync,
        }
    }

    /// Waiting tabs over `max` cut from the tail of the longest group, never from the first
    /// `protected` tabs of `session`'s group (the tabs just open); how many were cut. The same
    /// rule trims a file that is too full at load.
    pub(super) fn waiting_trimmed(&mut self, session: Uuid, protected: usize, max: usize) -> usize {
        let mut trimmed = 0;
        while self.dormant.values().map(Vec::len).sum::<usize>() > max {
            let spare =
                |id: &Uuid, len: usize| len - len.min(if *id == session { protected } else { 0 });
            let longest = self
                .dormant
                .iter()
                .map(|(id, tabs)| (spare(id, tabs.len()), *id))
                .max();
            let Some((_, id)) = longest.filter(|(room, _)| *room > 0) else {
                break;
            };
            if let Some(group) = self.dormant.get_mut(&id) {
                group.pop();
                trimmed += 1;
            }
        }
        self.hints_checked();
        trimmed
    }

    /// A hint that points past its group (cut, or reopened) is dropped.
    pub(super) fn hints_checked(&mut self) {
        let len = |id: &Uuid| self.dormant.get(id).map_or(0, Vec::len);
        if self.active_hint.is_some_and(|(id, at)| at >= len(&id)) {
            self.active_hint = None;
        }
        let kept = |hint: &(Uuid, SavedSplit)| {
            hint.1.secondary < len(&hint.0) && self.active_hint.is_some_and(|(id, _)| id == hint.0)
        };
        if self.split_hint.as_ref().is_some_and(|hint| !kept(hint)) {
            self.split_hint = None;
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
        self.waiting_notice_checked();
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

/// A tab was closed: one saved tab of a connected session that did not fit reopens (oldest first,
/// dormant, sending nothing, behind whatever the person is looking at), while at least one slot
/// stays free so a tab of the person's own can still be opened.
pub(super) fn refill(state: &mut AppState, closed_of: Option<Uuid>) {
    if state.restore.dormant.is_empty() {
        return;
    }
    let mut waiting: Vec<Uuid> = state
        .open
        .iter()
        .copied()
        .filter(|s| state.restore.dormant.contains_key(s))
        .filter(|s| state.status_of(*s) == Some(super::state::ConnStatus::Connected))
        .collect();
    // The connection of the tab that was closed goes first; the others follow in strip order.
    waiting.sort_by_key(|s| Some(*s) != closed_of);
    for session in waiting {
        if room_for(state, false, None) == 0 {
            break;
        }
        // One tab for one close.
        if reopened(state, session, true, false) > 0 {
            break;
        }
    }
    state.waiting_notice_checked();
}

#[inline(never)]
fn reopened(state: &mut AppState, session: Uuid, background: bool, initial: bool) -> usize {
    let mut saved = state.restore.dormant.remove(&session).unwrap_or_default();
    let active = state
        .restore
        .active_hint
        .filter(|(id, _)| *id == session)
        .map(|(_, at)| at);
    let split = state
        .restore
        .split_hint
        .filter(|(id, _)| *id == session)
        .map(|(_, split)| split);
    // Only the connect itself takes the front; a tab reopened later waits behind it, and leaves
    // one slot free so a tab of the person's own can still be opened.
    let front = initial && in_front(state, session, background);
    let reserve = usize::from(!initial);
    let mut room = room_for(state, front, active).saturating_sub(reserve);
    if !initial {
        room = room.min(1);
    }
    // What does not fit keeps waiting, in the file too: nothing saved is dropped for lack of room.
    let rest = saved.split_off(room.min(saved.len()));
    let opened_count = saved.len();
    // The hints follow their tabs among those that still wait.
    let kept_active = active
        .filter(|at| *at >= opened_count)
        .map(|at| at - opened_count);
    let kept_split = split.and_then(|s| {
        let secondary = s.secondary.checked_sub(opened_count)?;
        kept_active.map(|_| SavedSplit { secondary, ..s })
    });
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
    let shown_tab = active.and_then(at).filter(|_| front);
    if let Some(tab) = shown_tab {
        next = shown(next, tab);
        next = paired(next, tab, split, at);
    }
    // A hint is used up only when it was applied; otherwise it keeps its place.
    if active.is_some() {
        next.restore.active_hint = kept_active.map(|at| (session, at));
        // Reopened but not shown: the tab that now exists is still the saved front tab.
        if shown_tab.is_none() {
            next.restore.front_live = active.and_then(at).or(next.restore.front_live);
        }
    }
    if split.is_some() {
        next.restore.split_hint = match shown_tab {
            Some(_) => None,
            None => kept_split.map(|s| (session, s)),
        };
    }
    if initial && overflow > 0 {
        next = next.waiting_notice_raised();
    }
    *state = next;
    opened_count
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
    state.tabs = std::mem::take(&mut state.tabs).made_restored(
        tab,
        saved.text_dropped,
        saved.options_dropped,
    );
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
    /// saved) and for a runtime that stopped. The tabs that were just open (the working set, the
    /// front tab among it) come first; the ones already waiting follow. When the waiting tabs of
    /// all connections together are over their bound, the tails of the longest groups are cut,
    /// never the tabs just open, and a notice says so. The connection folding while it is in
    /// front takes the one slot the file has for the front tab and the split.
    pub(super) fn tabs_waiting(self, session: Uuid) -> AppState {
        let (state, trimmed) = self.tabs_folded(session, true);
        state.trimmed_notice_raised(trimmed)
    }

    fn tabs_folded(mut self, session: Uuid, fit: bool) -> (AppState, usize) {
        if !self.restore.restore || self.connection(session).is_none() {
            return (self, 0);
        }
        let open: Vec<(TabId, SavedTab)> = self
            .tabs
            .of_session(session)
            .into_iter()
            .filter_map(|tab| Some((tab, saved_tab(&self, tab)?)))
            .collect();
        if open.is_empty() {
            return (self, 0);
        }
        let place = |tab: TabId| open.iter().position(|(t, _)| *t == tab);
        let front = self.active_tab().and_then(place);
        let split = self.tabs.split.clone().and_then(|s| {
            let (primary, secondary) = (place(s.primary)?, place(s.secondary)?);
            (Some(primary) == front && primary != secondary).then_some((secondary, s))
        });
        let open_len = open.len();
        let old = self.restore.dormant.remove(&session).unwrap_or_default();
        let mut tabs: Vec<SavedTab> = open.into_iter().map(|(_, saved)| saved).collect();
        tabs.extend(old);
        // This connection's hints indexed the old waiting list: they move past the open tabs.
        if let Some((_, at)) = self
            .restore
            .active_hint
            .as_mut()
            .filter(|(id, _)| *id == session)
        {
            *at += open_len;
        }
        if let Some((_, s)) = self
            .restore
            .split_hint
            .as_mut()
            .filter(|(id, _)| *id == session)
        {
            s.secondary += open_len;
        }
        self.restore.dormant.insert(session, tabs);
        let trimmed = match fit {
            true => self.waiting_fitted(session, open_len, Some(session)),
            false => 0,
        };
        if let Some(at) = front {
            self.restore.active_hint = Some((session, at));
            self.restore.split_hint = split.map(|(secondary, s)| {
                let saved = SavedSplit {
                    secondary,
                    ratio: s.ratio,
                    orientation: s.orientation.into(),
                    secondary_first: s.secondary_first,
                };
                (session, saved)
            });
        }
        self.restore.hints_checked();
        (self, trimmed)
    }

    /// How many saved tabs of connected sessions wait for room.
    pub(super) fn waiting_for_room(&self) -> usize {
        self.open
            .iter()
            .filter(|s| self.status_of(**s) == Some(super::state::ConnStatus::Connected))
            .filter_map(|s| self.restore.dormant.get(s))
            .map(Vec::len)
            .sum()
    }

    /// `notice` shown, replacing any notice of its kind that is shown or queued.
    fn notice_replacing(mut self, is: fn(&Recovery) -> bool, notice: Recovery) -> AppState {
        self.pending_recovery.retain(|r| !is(r));
        match &self.banner {
            Some(Banner::Recovery(r)) if is(r) => {
                self.banner = Some(Banner::Recovery(notice));
                self
            }
            _ => self.with_recovery_notice(notice),
        }
    }

    /// A startup-style notice in the banner moved to the head of the queue, so a failure can
    /// take the banner without the notice being lost.
    pub(super) fn recovery_banner_queued(mut self) -> AppState {
        if let Some(Banner::Recovery(notice)) =
            self.banner.take_if(|b| matches!(b, Banner::Recovery(_)))
        {
            self.pending_recovery.insert(0, notice);
        }
        self
    }

    /// The notice that saved tabs are waiting for room, with how many wait now; a new one
    /// replaces any shown or queued.
    pub(super) fn waiting_notice_raised(self) -> AppState {
        let count = self.waiting_for_room();
        self.notice_replacing(is_waiting, Recovery::WorkspaceWaiting { count })
    }

    /// The waiting notice kept true: withdrawn once no tab of a connected session waits for room,
    /// and showing how many do while some do.
    pub(super) fn waiting_notice_checked(&mut self) {
        let left = self.waiting_for_room();
        let notice = Recovery::WorkspaceWaiting { count: left };
        if left == 0 {
            self.pending_recovery.retain(|r| !is_waiting(r));
            if matches!(&self.banner, Some(Banner::Recovery(r)) if is_waiting(r)) {
                self.banner = None;
            }
            return;
        }
        for queued in self.pending_recovery.iter_mut().filter(|r| is_waiting(r)) {
            *queued = notice.clone();
        }
        if matches!(&self.banner, Some(Banner::Recovery(r)) if is_waiting(r)) {
            self.banner = Some(Banner::Recovery(notice));
        }
    }

    /// One notice that `count` waiting tabs were cut, adding to one that is already up.
    pub(super) fn trimmed_notice_raised(self, count: usize) -> AppState {
        if count == 0 {
            return self;
        }
        let shown = match &self.banner {
            Some(Banner::Recovery(Recovery::WorkspaceTrimmed { count })) => *count,
            _ => 0,
        };
        let queued: usize = self
            .pending_recovery
            .iter()
            .map(|r| match r {
                Recovery::WorkspaceTrimmed { count } => *count,
                _ => 0,
            })
            .sum();
        let notice = Recovery::WorkspaceTrimmed {
            count: count + shown + queued,
        };
        self.notice_replacing(is_trimmed, notice)
    }

    /// The saved tabs of every open session that is open or waiting counted against the file's
    /// bound: the tabs of the strip that are saved, except those of `except`.
    fn live_saved_count(&self, except: Option<Uuid>) -> usize {
        self.visible_tabs()
            .into_iter()
            .filter(|tab| self.tabs.owner.get(tab).copied() != except)
            .filter(|tab| is_saved_tab(self, *tab))
            .count()
    }

    /// Waiting tabs cut until open and waiting tabs together fit the file; how many were cut (the
    /// first `protected` tabs of `session`'s group are never cut).
    pub(super) fn waiting_fitted(
        &mut self,
        session: Uuid,
        protected: usize,
        except_live: Option<Uuid>,
    ) -> usize {
        let max = MAX_SAVED_TABS.saturating_sub(self.live_saved_count(except_live));
        self.restore.waiting_trimmed(session, protected, max)
    }

    /// Every open session's tabs back to waiting (the sessions are all gone), with one notice for
    /// what was cut.
    pub(super) fn all_tabs_waiting(self) -> AppState {
        let open = self.open.clone();
        // Every session goes: no tab of the strip is left to count, so the cut is made once, at
        // the end, over all the waiting groups.
        let mut state = open
            .into_iter()
            .fold(self, |state, session| state.tabs_folded(session, false).0);
        let trimmed = state
            .restore
            .waiting_trimmed(Uuid::nil(), 0, MAX_SAVED_TABS);
        state.trimmed_notice_raised(trimmed)
    }
}

fn is_waiting(r: &Recovery) -> bool {
    matches!(r, Recovery::WorkspaceWaiting { .. })
}

fn is_trimmed(r: &Recovery) -> bool {
    matches!(r, Recovery::WorkspaceTrimmed { .. })
}

#[cfg(test)]
#[path = "workspace_restore_tests.rs"]
mod tests;
