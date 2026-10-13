//! Queueing field-level changes (slice 2c, ruling 8). Pure.
//!
//! One pending action per (pair, key) still holds: fields for a key replace any whole-document
//! change of that key (and the other way round), the same direction merges paths, the other
//! direction replaces. Fields only exist for rows of the `Different` category.
use std::collections::BTreeSet;

use curlybrace_core::compare::apply_fields::paths_conflict;
use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::PairId;
use curlybrace_core::compare::results::Category;

use super::field_select::PathKey;
use super::pending::{PendingChange, PendingList, Scope};
use super::selection::{OrdKey, RowAction};

/// What `PendingList::with_fields` did. Anything but `Queued` and `Merged` left the list as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldsQueued {
    /// A new fields item, replacing whatever the key had.
    Queued,
    /// The paths joined the key's item of the same direction.
    Merged,
    /// A category item already covers the tab.
    Swallowed,
    /// The paths (or the union with the queued ones) contain one that is a prefix of, or equal
    /// to, another; such a set cannot be written in one update.
    Conflict,
    /// No path was given.
    Empty,
}

/// The paths without repeats, in first-seen order.
fn distinct_paths(paths: Vec<Vec<PathSegment>>) -> Vec<Vec<PathSegment>> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|p| seen.insert(PathKey(p.clone())))
        .collect()
}

fn fields_key_matches(change: &PendingChange, pair: PairId, key: &KeyTuple) -> bool {
    change.same_tab(pair, Category::Different)
        && matches!(&change.scope, Scope::Fields { key: have, .. } if OrdKey(have.clone()) == OrdKey(key.clone()))
}

impl PendingList {
    /// Queues copying `paths` of the document `key` of the `Different` tab of `pair`; `action`
    /// must be one of `RowAction::offered_for_fields`. See `FieldsQueued` for the outcomes: a
    /// refused request (`Swallowed`, `Conflict`, `Empty`) returns the list unchanged.
    pub fn with_fields(
        &self,
        pair: PairId,
        key: &KeyTuple,
        action: RowAction,
        paths: Vec<Vec<PathSegment>>,
    ) -> (Self, FieldsQueued) {
        debug_assert!(
            RowAction::offered_for_fields().contains(&action),
            "{action:?} is not a field action"
        );
        let incoming = distinct_paths(paths);
        if incoming.is_empty() {
            return (self.clone(), FieldsQueued::Empty);
        }
        if paths_conflict(&incoming) {
            return (self.clone(), FieldsQueued::Conflict);
        }
        let covered = self
            .changes
            .iter()
            .any(|c| c.same_tab(pair, Category::Different) && c.scope == Scope::Category);
        if covered {
            return (self.clone(), FieldsQueued::Swallowed);
        }
        match self.position_of(pair, key) {
            Some(at) if self.changes[at].action == action => self.merged_at(at, key, incoming),
            _ => (
                self.replaced(pair, key, action, incoming),
                FieldsQueued::Queued,
            ),
        }
    }

    fn position_of(&self, pair: PairId, key: &KeyTuple) -> Option<usize> {
        self.changes
            .iter()
            .position(|c| fields_key_matches(c, pair, key))
    }

    /// The item at `at` with `incoming` appended after its own paths.
    fn merged_at(
        &self,
        at: usize,
        key: &KeyTuple,
        incoming: Vec<Vec<PathSegment>>,
    ) -> (Self, FieldsQueued) {
        let Scope::Fields { paths: have, .. } = &self.changes[at].scope else {
            return (self.clone(), FieldsQueued::Conflict);
        };
        let union = distinct_paths(have.iter().cloned().chain(incoming).collect());
        if paths_conflict(&union) {
            return (self.clone(), FieldsQueued::Conflict);
        }
        let mut changes = self.changes.clone();
        changes[at].scope = Scope::Fields {
            key: key.clone(),
            paths: union,
        };
        (Self { changes }, FieldsQueued::Merged)
    }

    /// The list with the key's whole-document and field items gone and a new fields item last.
    fn replaced(
        &self,
        pair: PairId,
        key: &KeyTuple,
        action: RowAction,
        paths: Vec<Vec<PathSegment>>,
    ) -> Self {
        let only: BTreeSet<OrdKey> = BTreeSet::from([OrdKey(key.clone())]);
        let mut changes = self.without_keys(pair, Category::Different, &only);
        changes.push(PendingChange {
            pair,
            category: Category::Different,
            action,
            scope: Scope::Fields {
                key: key.clone(),
                paths,
            },
        });
        Self { changes }
    }
}
