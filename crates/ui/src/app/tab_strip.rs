//! The order, ownership and recency of the open tabs, apart from what a tab shows. Pure: every op
//! returns the next strip.
use super::tabs::TabId;
use crate::viewmodel::split_layout::Orientation;
use crate::viewmodel::{AggregateState, CollectionTab, EditorState, FindBarState, ViewMode};
use curlybrace_core::store::Namespace;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// What a tab left behind when it was closed: its text and editor, never its results, so closing a
/// tab never loses typing for good and reopening it costs one read.
#[derive(Clone, Debug, PartialEq)]
pub struct ClosedTab {
    pub session: Uuid,
    pub ns: Option<Namespace>,
    pub find: FindBarState,
    pub tab: CollectionTab,
    pub view_mode: ViewMode,
    pub aggregate: Box<AggregateState>,
    pub editor: Option<EditorState>,
    /// Closed while it had not run since the connection was lost: it reopens the same way.
    pub dormant: bool,
    /// It was reopened from the saved session and had not run: it reopens the same way, with the
    /// same note.
    pub restored: bool,
    /// Its query text was too long to be saved.
    pub text_dropped: bool,
    /// The query that produced the page it showed.
    pub query: curlybrace_core::store::Query,
}

/// What the strip tells the person that is not about one tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabNotice {
    /// A new tab was refused: the app is at its tab limit.
    LimitReached,
}

/// Two panes: the tab being looked at and one partner (#159). The panes may be on different
/// connections. Focus moves between them by swapping `primary` and `secondary`; where each is
/// drawn does not change (`secondary_first`), so nothing moves on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    /// The active tab: the pane that has the keyboard and owns the dialogs.
    pub primary: TabId,
    /// The other pane.
    pub secondary: TabId,
    /// The first pane's share of the room the two have, clamped to a quarter to three quarters.
    pub ratio: f32,
    pub orientation: Orientation,
    /// The partner is drawn first (left or top) and the active tab second.
    pub secondary_first: bool,
}

/// Every open tab of the app: which session owns it, which tab of each session is the current one
/// (the one whose fields live in the session's view, see `tabs`), and in what order they were
/// opened and last used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabStrip {
    /// Strip order, left to right.
    pub order: Vec<TabId>,
    /// The tab the person is looking at.
    pub active: Option<TabId>,
    /// The session each tab belongs to.
    pub owner: BTreeMap<TabId, Uuid>,
    /// The tab of each session whose fields live in the session's view.
    pub current: BTreeMap<Uuid, TabId>,
    /// Most recently used first.
    pub mru: Vec<TabId>,
    /// Tabs whose connection was lost: they keep their text and send nothing until Run.
    pub dormant: BTreeSet<TabId>,
    /// Dormant tabs that came from the saved session (not from a lost connection): nothing has run
    /// in them yet, and their note says so.
    pub restored: BTreeSet<TabId>,
    /// Restored tabs whose query text was too long to be saved: their note says the text is gone.
    pub text_dropped: BTreeSet<TabId>,
    /// Restored tabs whose saved query options were not valid and were dropped.
    pub options_dropped: BTreeSet<TabId>,
    /// Current tabs that show less than the server holds (a parked tab keeps this mark in its own
    /// view; it moves here while the tab is current and back when it is parked again).
    pub stale: BTreeSet<TabId>,
    /// The last closed tabs, newest last (at most `tab_actions::CLOSED_KEPT`).
    pub closed: Vec<ClosedTab>,
    pub notice: Option<TabNotice>,
    /// The two panes, when the view is split. Never names a tab that is closed.
    pub split: Option<Split>,
    /// The number the next tab gets; it only grows, so a number is never reused.
    next: u64,
}

impl TabStrip {
    /// A new tab of `session`, placed after `after` (at the end when `None` or not in the strip).
    /// The session's first tab becomes its current one, and the strip's active one when it has
    /// none.
    pub fn opened_for(self, session: Uuid, after: Option<TabId>) -> (Self, TabId) {
        let tab = TabId(self.next);
        let at = after
            .and_then(|a| self.order.iter().position(|t| *t == a))
            .map_or(self.order.len(), |p| p + 1);
        let mut order = self.order;
        order.insert(at, tab);
        let mut owner = self.owner;
        owner.insert(tab, session);
        let mut current = self.current;
        current.entry(session).or_insert(tab);
        let mut mru = self.mru;
        mru.push(tab);
        (
            Self {
                order,
                active: self.active.or(Some(tab)),
                owner,
                current,
                mru,
                next: self.next + 1,
                ..self
            },
            tab,
        )
    }

    /// `tab` closed. The active tab moves to its right neighbour, else its left one. A session
    /// whose current tab closed has no current tab until the caller names one.
    pub fn closed(self, tab: TabId) -> Self {
        let Some(at) = self.order.iter().position(|t| *t == tab) else {
            return self;
        };
        let mut order = self.order;
        order.remove(at);
        let active = match self.active == Some(tab) {
            true => order
                .get(at)
                .or_else(|| at.checked_sub(1).and_then(|p| order.get(p))),
            false => self.active.as_ref(),
        }
        .copied();
        let mut owner = self.owner;
        let session = owner.remove(&tab);
        let current = self
            .current
            .into_iter()
            .filter(|(s, t)| Some(*s) != session || *t != tab)
            .collect();
        let mru = self.mru.into_iter().filter(|t| *t != tab).collect();
        let mut dormant = self.dormant;
        dormant.remove(&tab);
        let mut restored = self.restored;
        restored.remove(&tab);
        let mut text_dropped = self.text_dropped;
        text_dropped.remove(&tab);
        let mut options_dropped = self.options_dropped;
        options_dropped.remove(&tab);
        let mut stale = self.stale;
        stale.remove(&tab);
        // A pane that closes ends the split: it never names a tab that is gone.
        let split = self
            .split
            .filter(|s| s.primary != tab && s.secondary != tab);
        Self {
            order,
            active,
            owner,
            current,
            mru,
            dormant,
            restored,
            text_dropped,
            options_dropped,
            stale,
            split,
            ..self
        }
    }

    /// The split replaced by `split` (`None` ends it).
    pub fn with_split(self, split: Option<Split>) -> Self {
        Self { split, ..self }
    }

    /// Every tab of `session` closed (the session itself is gone).
    pub fn session_closed(self, session: Uuid) -> Self {
        let strip = self
            .of_session(session)
            .into_iter()
            .fold(self, |strip, tab| strip.closed(tab));
        // What it left behind cannot be reopened: there is nothing to open it on.
        let closed = strip.closed.into_iter().filter(|c| c.session != session);
        Self {
            closed: closed.collect(),
            ..strip
        }
    }

    /// `tab` is the one being looked at now: first in the recency list.
    pub fn activated(self, tab: TabId) -> Self {
        if !self.owner.contains_key(&tab) {
            return self;
        }
        let mru = std::iter::once(tab)
            .chain(self.mru.into_iter().filter(|t| *t != tab))
            .collect();
        Self {
            active: Some(tab),
            mru,
            ..self
        }
    }

    /// `tab` moved to position `to` (clamped to the end).
    pub fn moved(self, tab: TabId, to: usize) -> Self {
        let Some(from) = self.order.iter().position(|t| *t == tab) else {
            return self;
        };
        let mut order = self.order;
        order.remove(from);
        order.insert(to.min(order.len()), tab);
        Self { order, ..self }
    }

    /// No tab left, and no number given back: the next tab is still a new one.
    pub fn cleared(self) -> Self {
        Self {
            next: self.next,
            ..Self::default()
        }
    }

    /// `tabs` lost their connection: they run nothing until the person says so.
    pub fn made_dormant(self, tabs: impl IntoIterator<Item = TabId>) -> Self {
        let mut dormant = self.dormant;
        dormant.extend(tabs);
        Self { dormant, ..self }
    }

    /// `tab` is no longer dormant (its find was sent, or it was emptied).
    pub fn awakened(self, tab: TabId) -> Self {
        let mut dormant = self.dormant;
        dormant.remove(&tab);
        let mut restored = self.restored;
        restored.remove(&tab);
        let mut text_dropped = self.text_dropped;
        text_dropped.remove(&tab);
        let mut options_dropped = self.options_dropped;
        options_dropped.remove(&tab);
        Self {
            dormant,
            restored,
            text_dropped,
            options_dropped,
            ..self
        }
    }

    /// `tab` was reopened from the saved session: dormant, with nothing run yet.
    pub fn made_restored(self, tab: TabId, text_dropped: bool, options_dropped: bool) -> Self {
        let mut next = self.made_dormant([tab]);
        next.restored.insert(tab);
        if text_dropped {
            next.text_dropped.insert(tab);
        }
        if options_dropped {
            next.options_dropped.insert(tab);
        }
        next
    }

    /// `tab` (a current tab) is, or is no longer, known to show less than the server holds.
    pub fn with_stale(self, tab: TabId, stale: bool) -> Self {
        let mut marks = self.stale;
        match stale {
            true => marks.insert(tab),
            false => marks.remove(&tab),
        };
        Self {
            stale: marks,
            ..self
        }
    }

    /// A closed tab remembered (the newest `keep` of them are kept).
    pub fn with_closed(self, closed: ClosedTab, keep: usize) -> Self {
        let mut stack = self.closed;
        stack.push(closed);
        let excess = stack.len().saturating_sub(keep);
        stack.drain(..excess);
        Self {
            closed: stack,
            ..self
        }
    }

    /// The strip without its newest closed tab, and that tab.
    pub fn without_newest_closed(self) -> (Self, Option<ClosedTab>) {
        let mut stack = self.closed;
        let newest = stack.pop();
        (
            Self {
                closed: stack,
                ..self
            },
            newest,
        )
    }

    pub fn with_notice(self, notice: Option<TabNotice>) -> Self {
        Self { notice, ..self }
    }

    /// The tabs of `session`, in strip order.
    pub fn of_session(&self, session: Uuid) -> Vec<TabId> {
        self.order
            .iter()
            .copied()
            .filter(|t| self.owner.get(t) == Some(&session))
            .collect()
    }

    /// The tab of `session` whose fields live in the session's view.
    pub fn current_of(&self, session: Uuid) -> Option<TabId> {
        self.current.get(&session).copied()
    }

    /// `tab` is now `session`'s current tab.
    pub fn made_current(self, session: Uuid, tab: TabId) -> Self {
        let mut current = self.current;
        current.insert(session, tab);
        Self { current, ..self }
    }
}

#[cfg(test)]
#[path = "tab_strip_tests.rs"]
mod tests;
