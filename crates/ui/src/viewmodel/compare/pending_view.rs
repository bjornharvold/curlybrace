//! What the pending panel shows: the queued writes grouped by pair, each with the place it
//! writes, and why Review is closed when it is. Pure.
use curlybrace_core::compare::apply::ApplyKind;
use curlybrace_core::compare::protocol::PairId;
use uuid::Uuid;

use super::pending::Scope;
use super::review::Destination;
use super::rows::namespace_label;
use super::select_bar::ActionBlock;
use super::{CompareState, EntryId, RowAction};

/// One queued item, as listed.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingLine {
    /// Index into the pending list (what `RemovePending` takes).
    pub index: usize,
    pub action: RowAction,
    pub kind: ApplyKind,
    /// Documents the item stands for (a whole-tab item counts the stored total).
    pub documents: usize,
    /// The item covers every document of its tab.
    pub whole_tab: bool,
    pub destination: Destination,
}

/// The queued items of one pair.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingGroup {
    pub pair: PairId,
    pub entry: Option<EntryId>,
    pub lines: Vec<PendingLine>,
}

impl PendingGroup {
    pub fn documents(&self) -> usize {
        self.lines.iter().map(|l| l.documents).sum()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PendingView {
    pub groups: Vec<PendingGroup>,
}

impl PendingView {
    pub fn documents(&self) -> usize {
        self.groups.iter().map(PendingGroup::documents).sum()
    }
}

impl CompareState {
    /// The pending list grouped by pair in order of first appearance. `name_of` resolves a
    /// connection id to its display name.
    pub fn pending_view(&self, name_of: &dyn Fn(Uuid) -> String) -> PendingView {
        let mut groups: Vec<PendingGroup> = Vec::new();
        for (index, change) in self.pending.items().iter().enumerate() {
            let line = PendingLine {
                index,
                action: change.action,
                kind: change.action.kind_for(change.category),
                documents: match &change.scope {
                    Scope::Keys(keys) => keys.len(),
                    Scope::Fields { .. } => 1,
                    Scope::Category => self
                        .results
                        .get(&change.pair)
                        .map_or(0, |r| r.counts.of(change.category)),
                },
                whole_tab: change.scope == Scope::Category,
                destination: self.destination_of(change.pair, change.action, name_of),
            };
            match groups.iter_mut().find(|g| g.pair == change.pair) {
                Some(group) => group.lines.push(line),
                None => groups.push(PendingGroup {
                    pair: change.pair,
                    entry: self
                        .overview
                        .rows
                        .iter()
                        .find(|r| r.pair == change.pair)
                        .map(|r| r.entry),
                    lines: vec![line],
                }),
            }
        }
        PendingView { groups }
    }

    /// Where `action` on `pair` writes, from the facts of the run that produced its results.
    fn destination_of(
        &self,
        pair: PairId,
        action: RowAction,
        name_of: &dyn Fn(Uuid) -> String,
    ) -> Destination {
        use curlybrace_core::compare::apply::Direction;
        let facts = self
            .overview
            .rows
            .iter()
            .find(|r| r.pair == pair)
            .and_then(|r| r.facts.as_ref());
        let to_target = action.direction() == Direction::SourceToTarget;
        let connection = self
            .connections
            .map(|(source, target)| name_of(if to_target { target } else { source }))
            .unwrap_or_default();
        let ns = facts.map(|f| if to_target { &f.target } else { &f.source });
        Destination {
            connection,
            database: ns.map(|n| n.db.clone()).unwrap_or_default(),
            collection: ns.map(|n| n.collection.clone()).unwrap_or_default(),
        }
    }

    /// Review can be built: the cheap per-frame twin of `can_review` (no clone, no build).
    /// Everything the review needs is there: a session pair, run facts for every pending pair
    /// and stored results for every whole-tab item.
    pub fn review_resolvable(&self) -> bool {
        self.can_review_now()
            && self.connections.is_some()
            && self.pending.items().iter().all(|c| {
                self.review_entry(c.pair).is_some()
                    && (c.scope != Scope::Category || self.results.contains_key(&c.pair))
            })
    }

    /// The (source, target) names of the pair as its run saw them, so the pair label and the
    /// destination come from the same snapshot; the entry's current names when the run left
    /// no facts.
    pub fn pair_labels(&self, pair: PairId) -> Option<(String, String)> {
        let row = self.overview.rows.iter().find(|r| r.pair == pair)?;
        match &row.facts {
            Some(f) => Some((namespace_label(&f.source), namespace_label(&f.target))),
            None => self.entry_labels(row.entry),
        }
    }

    /// Why Review changes is closed; `None` when it can fire.
    pub fn review_block(&self) -> Option<ActionBlock> {
        if self.review_resolvable() {
            return None;
        }
        if self.apply.is_busy() {
            return Some(ActionBlock::Busy);
        }
        if self.is_running() {
            return Some(ActionBlock::Running);
        }
        if self.check_in_flight() {
            return Some(ActionBlock::Checking);
        }
        let blocked = self
            .pending
            .items()
            .iter()
            .find_map(|c| self.tick_block(c.pair));
        Some(blocked.unwrap_or(ActionBlock::NotCompleted))
    }
}
