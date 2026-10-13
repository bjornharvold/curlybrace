//! The queued writes of the Compare workspace, one action per (pair, key). Pure.
use std::cmp::Ordering;
use std::collections::BTreeSet;

use curlybrace_core::compare::apply::{ApplyKind, ApplyOp, Direction};
use curlybrace_core::compare::key::{KeyTuple, compare_tuples};
use curlybrace_core::compare::protocol::{BulkApply, PairId};
use curlybrace_core::compare::results::Category;

use super::selection::{OrdKey, RowAction};

/// Which rows a pending change covers.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Keys(Vec<KeyTuple>),
    /// Every row of the change's `category`, resolved by the runtime from the stored results.
    Category,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingChange {
    pub pair: PairId,
    pub category: Category,
    pub action: RowAction,
    pub scope: Scope,
}

impl PendingChange {
    fn same_tab(&self, pair: PairId, cat: Category) -> bool {
        self.pair == pair && self.category == cat
    }

    fn same_change(&self, other: &Self) -> bool {
        let same_scope = match (&self.scope, &other.scope) {
            (Scope::Category, Scope::Category) => true,
            (Scope::Keys(a), Scope::Keys(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(x, y)| compare_tuples(x, y) == Ordering::Equal)
            }
            _ => false,
        };
        self.pair == other.pair
            && self.category == other.category
            && self.action == other.action
            && same_scope
    }

    fn kind(&self) -> ApplyKind {
        self.action.kind_for(self.category)
    }

    fn direction(&self) -> Direction {
        self.action.direction()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PendingList {
    changes: Vec<PendingChange>,
}

impl PendingList {
    /// Queues `action` for `scope`, replacing whatever the same rows had. The count is the
    /// number of keys ignored because a category item already covers their tab.
    pub fn with_action(
        &self,
        pair: PairId,
        cat: Category,
        action: RowAction,
        scope: Scope,
    ) -> (Self, usize) {
        debug_assert!(
            RowAction::offered_for(cat).contains(&action),
            "{action:?} is not offered for {cat:?}"
        );
        let covered = self
            .changes
            .iter()
            .any(|c| c.same_tab(pair, cat) && c.scope == Scope::Category);
        match scope {
            Scope::Category => {
                let mut changes: Vec<_> = self
                    .changes
                    .iter()
                    .filter(|c| !c.same_tab(pair, cat))
                    .cloned()
                    .collect();
                changes.push(PendingChange {
                    pair,
                    category: cat,
                    action,
                    scope,
                });
                (Self { changes }, 0)
            }
            Scope::Keys(keys) => {
                let distinct = distinct_keys(keys);
                if covered {
                    return (self.clone(), distinct.len());
                }
                let incoming: BTreeSet<OrdKey> = distinct.iter().cloned().map(OrdKey).collect();
                let mut changes = self.without_keys(pair, cat, &incoming);
                changes.push(PendingChange {
                    pair,
                    category: cat,
                    action,
                    scope: Scope::Keys(distinct),
                });
                (Self { changes }, 0)
            }
        }
    }

    /// The items with `incoming` removed from the tab's key lists; emptied items go.
    fn without_keys(
        &self,
        pair: PairId,
        cat: Category,
        incoming: &BTreeSet<OrdKey>,
    ) -> Vec<PendingChange> {
        self.changes
            .iter()
            .filter_map(|c| match &c.scope {
                Scope::Keys(have) if c.same_tab(pair, cat) => {
                    let kept: Vec<KeyTuple> = have
                        .iter()
                        .filter(|k| !incoming.contains(&OrdKey((*k).clone())))
                        .cloned()
                        .collect();
                    (!kept.is_empty()).then(|| PendingChange {
                        scope: Scope::Keys(kept),
                        ..c.clone()
                    })
                }
                _ => Some(c.clone()),
            })
            .collect()
    }

    pub fn without(&self, index: usize) -> Self {
        Self {
            changes: self
                .changes
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, c)| c.clone())
                .collect(),
        }
    }

    pub fn without_pair(&self, pair: PairId) -> Self {
        Self {
            changes: self
                .changes
                .iter()
                .filter(|c| c.pair != pair)
                .cloned()
                .collect(),
        }
    }

    pub fn cleared(&self) -> Self {
        Self::default()
    }

    /// Both lists queue the same changes in the same order. Keys are compared by value
    /// (`compare_tuples`), so a key holding NaN equals its own clone, which `==` does not say.
    pub fn same_changes(&self, other: &Self) -> bool {
        self.changes.len() == other.changes.len()
            && self
                .changes
                .iter()
                .zip(&other.changes)
                .all(|(a, b)| a.same_change(b))
    }

    pub fn items(&self) -> &[PendingChange] {
        &self.changes
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// How many writes the list stands for; `totals` gives the size of a category.
    pub fn op_count(&self, totals: &dyn Fn(PairId, Category) -> usize) -> u64 {
        self.changes
            .iter()
            .map(|c| match &c.scope {
                Scope::Keys(keys) => keys.len() as u64,
                Scope::Category => totals(c.pair, c.category) as u64,
            })
            .sum()
    }

    /// The explicit operations and the category-wide items, for `CompareCommand::Apply`.
    pub fn to_command_parts(&self) -> (Vec<ApplyOp>, Vec<BulkApply>) {
        let mut ops = Vec::new();
        let mut bulk = Vec::new();
        for c in &self.changes {
            match &c.scope {
                Scope::Keys(keys) => ops.extend(keys.iter().map(|key| ApplyOp {
                    pair: c.pair,
                    key: key.clone(),
                    kind: c.kind(),
                    direction: c.direction(),
                })),
                Scope::Category => bulk.push(BulkApply {
                    pair: c.pair,
                    category: c.category,
                    kind: c.kind(),
                    direction: c.direction(),
                }),
            }
        }
        (ops, bulk)
    }
}

/// The keys without repeats (by `compare_tuples` equality), in first-seen order.
fn distinct_keys(keys: Vec<KeyTuple>) -> Vec<KeyTuple> {
    let mut seen = BTreeSet::new();
    keys.into_iter()
        .filter(|k| seen.insert(OrdKey(k.clone())))
        .collect()
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
