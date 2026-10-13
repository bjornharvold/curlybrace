//! View-models of the Compare workspace. Pure and egui-free.
pub mod checks;
pub mod detail;
pub mod duplicates;
pub mod focus;
pub mod overview;
pub mod pairing_view;
pub mod pairs;
pub mod preflight;
pub mod results;
pub mod setup;

use std::collections::BTreeMap;

use curlybrace_core::compare::protocol::{PairId, PairSpec, RunId, SessionId};
use uuid::Uuid;

pub use checks::{CheckQueue, EntryCheck};
pub use detail::{ChangeLine, DetailView, canonical_json};
pub use duplicates::{DuplicateRow, DuplicatesView};
pub use focus::RunView;
pub use overview::{ActiveRun, PairStatus, RunOverview, RunRow, Totals};
pub use pairing_view::{DatabasePairing, PairingRow};
pub use pairs::{EntryId, PairEntry, PairList, PairOverrides};
pub use preflight::{PreflightMessage, PreflightView};
pub use results::{Counts, PAGE_SIZE, ResultsView, category_key, key_label};
pub use setup::{Pick, RulesForm, ScopeKind, SetupError, SetupForm, SideForm};

/// Where the compare session stands. It owns its own connections, so the browsing connection
/// coming and going never changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Opening(SessionId),
    Ready(SessionId),
}

impl SessionPhase {
    pub fn id(&self) -> SessionId {
        match self {
            Self::Opening(id) | Self::Ready(id) => *id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompareState {
    pub session: Option<SessionPhase>,
    /// The connections the current session was opened for.
    pub connections: Option<(Uuid, Uuid)>,
    /// Ids are chosen here and never reused, so a late event can never match a newer session,
    /// run or pair.
    pub next_session: SessionId,
    pub next_run: RunId,
    /// The next runtime `PairId`: every run and every check probe takes a fresh one.
    pub next_pair: PairId,
    /// The shared defaults and the pickers feeding Add pair.
    pub setup: SetupForm,
    pub checks: CheckQueue,
    /// Probe of a check an edit made stale and whose answer has not arrived yet. A failure
    /// carries no probe and the runtime may still be working on the old check, so while this is
    /// set no new check is sent (one `Check` in flight): the next answer or failure is the
    /// orphan's and is dropped.
    pub orphaned_check: Option<PairId>,
    pub overview: RunOverview,
    pub pairs: PairList,
    pub pairing: DatabasePairing,
    /// The results of every finished pair that has not been discarded, by runtime pair.
    pub results: BTreeMap<PairId, ResultsView>,
    /// The pair whose results are shown.
    pub open_pair: Option<PairId>,
    pub detail: Option<DetailView>,
    /// The duplicate-keys panel, offered after a run failed on a duplicate key.
    pub duplicates: DuplicatesView,
}

impl Default for CompareState {
    fn default() -> Self {
        Self {
            session: None,
            connections: None,
            next_session: 1,
            next_run: 1,
            next_pair: 1,
            setup: SetupForm::default(),
            checks: CheckQueue::default(),
            orphaned_check: None,
            overview: RunOverview::default(),
            pairs: PairList::default(),
            pairing: DatabasePairing::default(),
            results: BTreeMap::new(),
            open_pair: None,
            detail: None,
            duplicates: DuplicatesView::default(),
        }
    }
}

impl CompareState {
    /// Forgets everything tied to the current session; the id counters survive.
    pub fn session_dropped(self) -> Self {
        Self {
            next_session: self.next_session,
            next_run: self.next_run,
            next_pair: self.next_pair,
            ..Self::default()
        }
    }
}

impl CompareState {
    /// Whether Connect may fire for the picked pair: both picked, different from what is open,
    /// and no open in progress.
    pub fn can_open(&self, draft: (Option<Uuid>, Option<Uuid>)) -> bool {
        let opening = matches!(self.session, Some(SessionPhase::Opening(_)));
        match draft {
            (Some(source), Some(target)) => !opening && self.connections != Some((source, target)),
            _ => false,
        }
    }

    pub fn can_close(&self) -> bool {
        self.session.is_some()
    }

    /// Finished results can be thrown away (never while a run is in flight).
    pub fn can_discard(&self) -> bool {
        self.is_ready() && !self.is_running() && self.overview.any_settled()
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.session, Some(SessionPhase::Ready(_)))
    }

    pub fn is_running(&self) -> bool {
        self.overview.is_running()
    }

    /// Everything wrong with the setup, in display order: the form's own issues, then whether
    /// both sides are the very same collection (which needs the session's connections).
    pub fn issues(&self) -> Vec<SetupError> {
        let mut issues = self.setup.issues();
        if issues.is_empty() && self.one_connection() && self.setup.same_namespace_and_filter() {
            issues.push(SetupError::SameCollection);
        }
        issues
    }

    pub(super) fn one_connection(&self) -> bool {
        self.connections.is_some_and(|(s, t)| s == t)
    }

    /// The spec of an entry under the runtime id `id`; `None` for an unknown entry.
    pub fn spec_for(&self, entry: EntryId, id: PairId) -> Option<Result<PairSpec, SetupError>> {
        let found = self.pairs.get(entry)?;
        Some(found.spec(id, &self.setup, self.one_connection()))
    }

    /// The included entries whose spec builds: what a check queues.
    pub fn checkable_entries(&self) -> Vec<EntryId> {
        let same = self.one_connection();
        self.pairs
            .entries
            .iter()
            .filter(|e| e.included && e.issues(&self.setup, same).is_empty())
            .map(|e| e.id)
            .collect()
    }

    /// Whether the Check button may fire: a session, nothing in flight and something to check.
    pub fn can_check(&self) -> bool {
        self.is_ready() && !self.check_in_flight() && !self.is_running() && self.has_valid_entry()
    }

    /// Whether Run may fire: at least one included entry is usable and every included entry has
    /// a fresh verdict that allows it.
    pub fn can_start(&self) -> bool {
        self.start_gate(false)
    }

    /// Whether Run passing may fire: like `can_start`, but refused entries are skipped, so only
    /// one passing entry has to exist.
    pub fn can_start_passing(&self) -> bool {
        self.start_gate(true)
    }

    /// Whether Cancel may fire: a run is in flight and no cancel was sent for it yet.
    pub fn can_cancel(&self) -> bool {
        self.overview.can_cancel()
    }

    /// An edit: every verdict is dropped, a check still out becomes the orphan and a duplicates
    /// lookup in flight goes stale.
    pub fn invalidated(self) -> Self {
        let orphan = self
            .checks
            .in_flight()
            .map(|(_, probe)| probe)
            .or(self.orphaned_check);
        Self {
            checks: self.checks.invalidated(),
            orphaned_check: orphan,
            duplicates: self.duplicates.superseded(),
            ..self
        }
    }

    /// The results of `pairs` are gone; the open pair and its detail go with them.
    pub fn without_results(self, pairs: &[PairId]) -> Self {
        let closed = self.open_pair.is_some_and(|open| pairs.contains(&open));
        Self {
            results: self
                .results
                .into_iter()
                .filter(|(pair, _)| !pairs.contains(pair))
                .collect(),
            open_pair: self.open_pair.filter(|open| !pairs.contains(open)),
            detail: if closed { None } else { self.detail },
            ..self
        }
    }

    /// A `Rows` request failed. The failure names no pair, so every view that waits for a page
    /// is marked failed (retry works once the pair is opened); the flag says whether any did
    /// (none: the failure belongs to a request we dropped ourselves, e.g. by Discard).
    pub fn rows_failed_all(self) -> (Self, bool) {
        let any = self.results.values().any(ResultsView::is_loading);
        let results = self
            .results
            .into_iter()
            .map(|(pair, view)| (pair, view.rows_failed()))
            .collect();
        (Self { results, ..self }, any)
    }

    /// The results of `pair` passed through `change`; unknown pairs change nothing.
    pub fn results_updated(
        self,
        pair: PairId,
        change: impl FnOnce(ResultsView) -> ResultsView,
    ) -> Self {
        match self.results.get(&pair).cloned() {
            Some(view) => Self {
                results: self
                    .results
                    .into_iter()
                    .chain([(pair, change(view))])
                    .collect(),
                ..self
            },
            None => self,
        }
    }

    fn included_ids(&self) -> Vec<EntryId> {
        self.pairs
            .entries
            .iter()
            .filter(|e| e.included)
            .map(|e| e.id)
            .collect()
    }

    pub fn has_valid_entry(&self) -> bool {
        let same = self.one_connection();
        self.pairs
            .entries
            .iter()
            .any(|e| e.included && e.issues(&self.setup, same).is_empty())
    }

    fn start_gate(&self, passing_only: bool) -> bool {
        let any_valid = self.has_valid_entry();
        // Run passing only has to wait for the entries that can be checked at all; an included
        // entry with an issue is never queued and must not block the passing ones.
        let included = if passing_only {
            self.checkable_entries()
        } else {
            self.included_ids()
        };
        let idle = self.is_ready() && !self.is_running() && !self.check_in_flight();
        if !idle || !any_valid || !self.checks.all_settled(&included) {
            return false;
        }
        let passes = |id: &EntryId| self.checks.allows_start(*id);
        match passing_only {
            true => included.iter().any(passes),
            false => !included.is_empty() && included.iter().all(passes),
        }
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;
