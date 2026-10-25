//! Conflict policies and pure batch planning for copy. No I/O: the engine looks up which
//! `_id`s already exist in the target and asks this module what to write.

use bson::{Bson, Document};

use crate::compare::key::compare_bson;
use crate::store::WriteOp;

/// What happens when a copied document's `_id` already exists in the target.
/// Nothing is ever deleted from the target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ConflictPolicy {
    /// Insert new documents only; existing ones are left alone.
    #[default]
    Skip,
    /// Replace existing documents (whole document); never insert new ones.
    Overwrite,
    /// Replace existing documents and insert new ones.
    Upsert,
}

/// The `_id`s already present in the target for one batch. `Bson` is not `Hash`, so ids are
/// kept sorted and matched by `compare_bson` (which treats `Int32(1)` and `Int64(1)` as equal).
#[derive(Clone, Debug, Default)]
pub struct ExistingIds {
    sorted: Vec<Bson>,
}

impl ExistingIds {
    pub fn from_ids(ids: Vec<Bson>) -> Self {
        let mut sorted = ids;
        sorted.sort_by(compare_bson);
        Self { sorted }
    }

    pub fn contains(&self, id: &Bson) -> bool {
        self.sorted
            .binary_search_by(|probe| compare_bson(probe, id))
            .is_ok()
    }
}

/// The writes for one batch and how many documents needed none.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchPlan {
    pub ops: Vec<WriteOp>,
    pub skipped: u64,
}

/// One op per document that the policy lets through; the rest are counted as skipped.
/// A document without an `_id` is never in the target, so it is a new document.
pub fn plan_batch(
    policy: ConflictPolicy,
    docs: Vec<Document>,
    existing: &ExistingIds,
) -> BatchPlan {
    let total = docs.len() as u64;
    let ops: Vec<WriteOp> = docs
        .into_iter()
        .filter_map(|doc| plan_one(policy, doc, existing))
        .collect();
    let skipped = total - ops.len() as u64;
    BatchPlan { ops, skipped }
}

fn plan_one(policy: ConflictPolicy, doc: Document, existing: &ExistingIds) -> Option<WriteOp> {
    let id = doc.get("_id").cloned();
    let exists = id.as_ref().is_some_and(|id| existing.contains(id));
    match (policy, exists, id) {
        (ConflictPolicy::Skip | ConflictPolicy::Upsert, false, _) => Some(WriteOp::Insert { doc }),
        (ConflictPolicy::Overwrite | ConflictPolicy::Upsert, true, Some(id)) => {
            Some(WriteOp::Replace { id, doc })
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
