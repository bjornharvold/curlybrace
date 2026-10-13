//! The queued writes of the Compare workspace, one action per (pair, key). Pure.
use std::cmp::Ordering;
use std::collections::BTreeSet;

use curlybrace_core::compare::apply::{ApplyKind, ApplyOp, Direction};
use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::key::{KeyTuple, compare_tuples};
use curlybrace_core::compare::protocol::{BulkApply, PairId};
use curlybrace_core::compare::results::Category;

use super::field_select::PathKey;
use super::pending_fields::FieldsQueued;
use super::selection::{OrdKey, RowAction};

/// Which rows a pending change covers.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Keys(Vec<KeyTuple>),
    /// Every row of the change's `category`, resolved by the runtime from the stored results.
    Category,
    /// Chosen fields of one differing document: one write per document, with all its paths.
    Fields {
        key: KeyTuple,
        paths: Vec<Vec<PathSegment>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingChange {
    pub pair: PairId,
    pub category: Category,
    pub action: RowAction,
    pub scope: Scope,
}

impl PendingChange {
    pub(super) fn same_tab(&self, pair: PairId, cat: Category) -> bool {
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
            (Scope::Fields { key: ka, paths: pa }, Scope::Fields { key: kb, paths: pb }) => {
                compare_tuples(ka, kb) == Ordering::Equal && same_paths(pa, pb)
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
    pub(super) changes: Vec<PendingChange>,
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
            matches!(scope, Scope::Fields { .. }) || RowAction::offered_for(cat).contains(&action),
            "{action:?} is not offered for {cat:?}"
        );
        let covered = self
            .changes
            .iter()
            .any(|c| c.same_tab(pair, cat) && c.scope == Scope::Category);
        match scope {
            Scope::Fields { key, paths } => {
                // Field items have their own rules; a request a category item covers is ignored.
                // Callers that need the Conflict/Empty/Swallowed feedback use `with_fields`.
                debug_assert_eq!(cat, Category::Different, "field items are Different-only");
                let (list, outcome) = self.with_fields(pair, &key, action, paths);
                (list, usize::from(outcome == FieldsQueued::Swallowed))
            }
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
            Scope::Keys(keys) => self.with_keys(pair, cat, action, keys, covered),
        }
    }

    /// Queues explicit keys: the keys a category item covers are ignored (and counted), the others
    /// replace whatever the same rows had.
    fn with_keys(
        &self,
        pair: PairId,
        cat: Category,
        action: RowAction,
        keys: Vec<KeyTuple>,
        covered: bool,
    ) -> (Self, usize) {
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

    /// The items with `incoming` removed from the tab's key lists; emptied items go.
    pub(super) fn without_keys(
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
                // A whole-document change and a field change of one key replace each other.
                Scope::Fields { key, .. }
                    if c.same_tab(pair, cat) && incoming.contains(&OrdKey(key.clone())) =>
                {
                    None
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
                Scope::Fields { .. } => 1,
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
                    fields: Vec::new(),
                })),
                Scope::Fields { key, paths } => ops.push(ApplyOp {
                    pair: c.pair,
                    key: key.clone(),
                    kind: c.kind(),
                    direction: c.direction(),
                    fields: paths.clone(),
                }),
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

/// Both lists hold the same paths, whatever their order.
fn same_paths(a: &[Vec<PathSegment>], b: &[Vec<PathSegment>]) -> bool {
    let set = |paths: &[Vec<PathSegment>]| -> BTreeSet<PathKey> {
        paths.iter().cloned().map(PathKey).collect()
    };
    a.len() == b.len() && set(a) == set(b)
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pending_fields_tests.rs"]
mod fields_tests;
