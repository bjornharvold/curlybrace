//! Which of the Compare buttons may fire. The decisions live here; the reducer applies the same
//! gates, so a click the views allow is a click the reducer honours.
//!
//! Run semantics: Run covers the VALID included entries (those whose spec builds) and needs each
//! of them to have passed its check; invalid entries are skipped and shown with their issue.
//! Run passing needs only one passing valid entry, so an entry that was refused or whose check
//! failed never blocks it.
use super::CompareState;
use super::checks::EntryCheck;
use super::digest::PairsDigest;
use super::overview::PairStatus;
use super::pairs::EntryId;
use curlybrace_core::compare::protocol::PairId;

/// The enabled state of the workspace's buttons for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gates {
    pub check: bool,
    pub start: bool,
    pub start_passing: bool,
    pub cancel: bool,
    /// Review changes may fire: see `CompareState::can_review`.
    pub review: bool,
    /// Some included entry will not run under Run: invalid, its check failed, or it was
    /// refused. Run passing is offered.
    pub some_not_passing: bool,
}

impl CompareState {
    /// What the pair list and the defaults make of every entry (not free: memoise per frame).
    pub fn digest(&self) -> PairsDigest {
        PairsDigest::of(&self.pairs, &self.setup, self.one_connection())
    }

    /// The buttons' gates for `digest`, which must be this state's digest.
    pub fn gates(&self, digest: &PairsDigest) -> Gates {
        let idle = self.is_ready()
            && !self.is_running()
            && !self.check_in_flight()
            && !self.apply.is_busy();
        Gates {
            check: idle && !digest.valid.is_empty(),
            start: idle && self.start_gate(digest, false),
            start_passing: idle && self.start_gate(digest, true),
            cancel: self.can_cancel(),
            review: self.can_review(),
            some_not_passing: self.some_not_passing(digest),
        }
    }

    fn some_not_passing(&self, digest: &PairsDigest) -> bool {
        self.pairs.entries.iter().enumerate().any(|(index, e)| {
            e.included
                && (digest.issue_at(index).is_some()
                    || matches!(self.checks.entries.get(&e.id), Some(EntryCheck::Failed))
                    || self.checks.verdict(e.id).is_some_and(|v| !v.allows_start()))
        })
    }

    /// Whether the Check button may fire: a session, nothing in flight and something to check.
    pub fn can_check(&self) -> bool {
        self.gates(&self.digest()).check
    }

    /// Whether Run may fire: every valid included entry has a fresh passing verdict.
    pub fn can_start(&self) -> bool {
        self.gates(&self.digest()).start
    }

    /// Whether Run passing may fire: one valid included entry passed and none still waits.
    pub fn can_start_passing(&self) -> bool {
        self.gates(&self.digest()).start_passing
    }

    /// The included entries whose spec builds: what a check queues and what a run covers.
    pub fn checkable_entries(&self) -> Vec<EntryId> {
        self.digest().valid
    }

    /// Remove may fire for `entry`: it exists, no run is active, no check is queued or in
    /// flight, and its own row is not queued or running.
    pub fn can_remove(&self, entry: EntryId) -> bool {
        self.pairs.get(entry).is_some()
            && !self.is_running()
            && !self.check_in_flight()
            && !self.apply.is_busy()
            && !self.overview.entry_is_live(entry)
    }

    /// The state allows a review: a ready session, something pending, every pending pair
    /// completed and not stale, and no run, check or apply in the way.
    pub(super) fn can_review_now(&self) -> bool {
        self.is_ready()
            && !self.pending.is_empty()
            && !self.is_running()
            && !self.check_in_flight()
            && !self.apply.is_busy()
            && self
                .pending
                .items()
                .iter()
                .all(|c| self.pair_is_applicable(c.pair))
    }

    /// The pair completed and no apply has touched it since: rows may be ticked and queued.
    pub fn pair_is_applicable(&self, pair: PairId) -> bool {
        self.overview
            .rows
            .iter()
            .find(|r| r.pair == pair)
            .is_some_and(|r| matches!(r.status, PairStatus::Completed(_)) && !r.applied)
    }

    /// Discard may fire for `entry`: its latest run has settled (not live, not discarded yet)
    /// and no apply is reviewing or running. The reducer's discard and the row's button both
    /// use this.
    pub fn can_discard_entry(&self, entry: EntryId) -> bool {
        !self.apply.is_busy()
            && self
                .overview
                .row_for(entry)
                .is_some_and(|r| r.status.is_settled())
    }

    fn start_gate(&self, digest: &PairsDigest, passing_only: bool) -> bool {
        let ids = &digest.valid;
        if ids.is_empty() || !self.checks.all_settled(ids) {
            return false;
        }
        let passes = |id: &EntryId| self.checks.allows_start(*id);
        match passing_only {
            true => ids.iter().any(passes),
            false => ids.iter().all(passes),
        }
    }
}
