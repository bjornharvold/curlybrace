//! Which of the Compare buttons may fire. The decisions live here; the reducer applies the same
//! gates, so a click the views allow is a click the reducer honours.
use super::CompareState;
use super::digest::PairsDigest;
use super::pairs::EntryId;

/// The enabled state of the workspace's buttons for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gates {
    pub check: bool,
    pub start: bool,
    pub start_passing: bool,
    pub cancel: bool,
    /// Some included entry was checked and refused: Run passing is offered.
    pub some_refused: bool,
}

impl CompareState {
    /// What the pair list and the defaults make of every entry (not free: memoise per frame).
    pub fn digest(&self) -> PairsDigest {
        PairsDigest::of(&self.pairs, &self.setup, self.one_connection())
    }

    /// The buttons' gates for `digest`, which must be this state's digest.
    pub fn gates(&self, digest: &PairsDigest) -> Gates {
        let idle = self.is_ready() && !self.is_running() && !self.check_in_flight();
        let included = self.included_ids();
        Gates {
            check: self.is_ready()
                && !self.check_in_flight()
                && !self.is_running()
                && !digest.valid.is_empty(),
            start: idle && self.start_gate(digest, &included, false),
            start_passing: idle && self.start_gate(digest, &digest.valid, true),
            cancel: self.can_cancel(),
            some_refused: included
                .iter()
                .any(|id| self.checks.verdict(*id).is_some_and(|v| !v.allows_start())),
        }
    }

    /// Whether the Check button may fire: a session, nothing in flight and something to check.
    pub fn can_check(&self) -> bool {
        self.gates(&self.digest()).check
    }

    /// Whether Run may fire: at least one included entry is usable and every included entry has
    /// a fresh verdict that allows it.
    pub fn can_start(&self) -> bool {
        self.gates(&self.digest()).start
    }

    /// Whether Run passing may fire: like `can_start`, but refused entries are skipped, so only
    /// one passing entry has to exist.
    pub fn can_start_passing(&self) -> bool {
        self.gates(&self.digest()).start_passing
    }

    /// The included entries whose spec builds: what a check queues.
    pub fn checkable_entries(&self) -> Vec<EntryId> {
        self.digest().valid
    }

    pub fn has_valid_entry(&self) -> bool {
        !self.digest().valid.is_empty()
    }

    fn included_ids(&self) -> Vec<EntryId> {
        self.pairs
            .entries
            .iter()
            .filter(|e| e.included)
            .map(|e| e.id)
            .collect()
    }

    /// Run passing only has to wait for the entries that can be checked at all (`ids` are the
    /// valid ones): an included entry with an issue is never queued and must not block the
    /// passing ones. Run waits for every included entry.
    fn start_gate(&self, digest: &PairsDigest, ids: &[EntryId], passing_only: bool) -> bool {
        if digest.valid.is_empty() || !self.checks.all_settled(ids) {
            return false;
        }
        let passes = |id: &EntryId| self.checks.allows_start(*id);
        match passing_only {
            true => ids.iter().any(passes),
            false => !ids.is_empty() && ids.iter().all(passes),
        }
    }
}
