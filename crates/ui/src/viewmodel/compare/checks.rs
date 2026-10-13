//! The preflight checks of the pair entries: one `Check` in flight at a time, each under a probe
//! `PairId` so an answer names its entry and a stale answer is recognisable. Pure; every step
//! returns a new queue.
use std::collections::BTreeMap;

use curlybrace_core::compare::protocol::{PairId, Preflight};

use super::pairs::EntryId;
use super::preflight::PreflightView;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum EntryCheck {
    /// Never checked, or edited since.
    #[default]
    Unchecked,
    Queued,
    Checking(PairId),
    Done(PreflightView, Preflight),
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckQueue {
    pub entries: BTreeMap<EntryId, EntryCheck>,
}

impl CheckQueue {
    pub fn queued(ids: &[EntryId]) -> Self {
        Self {
            entries: ids.iter().map(|id| (*id, EntryCheck::Queued)).collect(),
        }
    }

    /// The entry whose check is in flight, with the probe it was sent under.
    pub fn in_flight(&self) -> Option<(EntryId, PairId)> {
        self.entries.iter().find_map(|(id, check)| match check {
            EntryCheck::Checking(probe) => Some((*id, *probe)),
            _ => None,
        })
    }

    /// The next entry to send; none while a check is outstanding (one `Check` at a time).
    pub fn next_to_send(&self) -> Option<EntryId> {
        if self.in_flight().is_some() {
            return None;
        }
        self.entries
            .iter()
            .find_map(|(id, check)| (*check == EntryCheck::Queued).then_some(*id))
    }

    /// Whether any entry still waits for, or is in, a check.
    pub fn has_pending(&self) -> bool {
        self.entries
            .values()
            .any(|c| matches!(c, EntryCheck::Queued | EntryCheck::Checking(_)))
    }

    pub fn sending(self, id: EntryId, probe: PairId) -> Self {
        self.map_entry(id, |check| match check {
            EntryCheck::Queued => EntryCheck::Checking(probe),
            other => other,
        })
    }

    /// Stores the verdict of the entry checked under `probe`; `false` when `probe` is not in
    /// flight (the entry was edited, removed or the queue replaced since).
    pub fn answered(self, probe: PairId, answer: Preflight) -> (Self, bool) {
        match self.in_flight() {
            Some((id, current)) if current == probe => {
                let done = EntryCheck::Done(PreflightView::from(&answer), answer);
                (self.map_entry(id, |_| done), true)
            }
            _ => (self, false),
        }
    }

    pub fn failed(self, probe: PairId) -> Self {
        match self.in_flight() {
            Some((id, current)) if current == probe => self.map_entry(id, |_| EntryCheck::Failed),
            _ => self,
        }
    }

    /// The entry cannot be sent (its spec no longer builds): it ends as failed, so the queue
    /// moves on instead of waiting for an answer that will never come.
    pub fn skipped(self, id: EntryId) -> Self {
        self.map_entry(id, |check| match check {
            EntryCheck::Queued => EntryCheck::Failed,
            other => other,
        })
    }

    pub fn verdict(&self, id: EntryId) -> Option<&PreflightView> {
        match self.entries.get(&id) {
            Some(EntryCheck::Done(view, _)) => Some(view),
            _ => None,
        }
    }

    /// The counts and index status behind the verdict.
    pub fn preflight(&self, id: EntryId) -> Option<&Preflight> {
        match self.entries.get(&id) {
            Some(EntryCheck::Done(_, answer)) => Some(answer),
            _ => None,
        }
    }

    pub fn all_settled(&self, ids: &[EntryId]) -> bool {
        ids.iter().all(|id| {
            matches!(
                self.entries.get(id),
                Some(EntryCheck::Done(..) | EntryCheck::Failed)
            )
        })
    }

    pub fn allows_start(&self, id: EntryId) -> bool {
        self.verdict(id).is_some_and(PreflightView::allows_start)
    }

    /// An edit: every verdict is dropped and an answer still in flight becomes stale.
    pub fn invalidated(self) -> Self {
        Self {
            entries: self
                .entries
                .into_keys()
                .map(|id| (id, EntryCheck::Unchecked))
                .collect(),
        }
    }

    /// Replaces one entry's state; an unknown id changes nothing. `self` is owned, so this
    /// builds the next queue rather than editing a shared one.
    fn map_entry(mut self, id: EntryId, change: impl FnOnce(EntryCheck) -> EntryCheck) -> Self {
        if let Some(check) = self.entries.remove(&id) {
            self.entries.insert(id, change(check));
        }
        self
    }
}

#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;
