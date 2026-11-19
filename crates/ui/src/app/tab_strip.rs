//! The order, ownership and recency of the open tabs, apart from what a tab shows. Pure: every op
//! returns the next strip.
use super::tabs::TabId;
use std::collections::BTreeMap;
use uuid::Uuid;

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
        Self {
            order,
            active,
            owner,
            current,
            mru,
            next: self.next,
        }
    }

    /// Every tab of `session` closed (the session itself is gone).
    pub fn session_closed(self, session: Uuid) -> Self {
        self.of_session(session)
            .into_iter()
            .fold(self, |strip, tab| strip.closed(tab))
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
