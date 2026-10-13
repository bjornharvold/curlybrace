//! The queued writes of the Compare workspace, one action per (pair, key). Pure.
use curlybrace_core::compare::apply::{ApplyKind, ApplyOp, Direction};
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{BulkApply, PairId};
use curlybrace_core::compare::results::Category;

use super::selection::RowAction;

/// Which rows a pending change covers.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Keys(Vec<KeyTuple>),
    /// Every row of the category, resolved by the runtime from the stored results.
    Category(Category),
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
        let covered = self
            .changes
            .iter()
            .any(|c| c.same_tab(pair, cat) && matches!(c.scope, Scope::Category(_)));
        let added = PendingChange {
            pair,
            category: cat,
            action,
            scope,
        };
        match &added.scope {
            Scope::Category(_) => {
                let mut changes: Vec<_> = self
                    .changes
                    .iter()
                    .filter(|c| !c.same_tab(pair, cat))
                    .cloned()
                    .collect();
                changes.push(added);
                (Self { changes }, 0)
            }
            Scope::Keys(keys) if covered => (self.clone(), keys.len()),
            Scope::Keys(keys) => {
                let mut changes = self.without_keys(pair, cat, keys);
                changes.push(added);
                (Self { changes }, 0)
            }
        }
    }

    /// The items with `keys` of the tab removed from their key lists; emptied items go.
    fn without_keys(&self, pair: PairId, cat: Category, keys: &[KeyTuple]) -> Vec<PendingChange> {
        self.changes
            .iter()
            .filter_map(|c| match &c.scope {
                Scope::Keys(have) if c.same_tab(pair, cat) => {
                    let kept: Vec<KeyTuple> =
                        have.iter().filter(|k| !keys.contains(k)).cloned().collect();
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
                Scope::Category(cat) => totals(c.pair, *cat) as u64,
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
                Scope::Category(category) => bulk.push(BulkApply {
                    pair: c.pair,
                    category: *category,
                    kind: c.kind(),
                    direction: c.direction(),
                }),
            }
        }
        (ops, bulk)
    }
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
