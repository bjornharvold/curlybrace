//! Tests for conflict policies and batch planning.

use super::*;
use crate::test_support::mock_objects::mock_doc_with_id;
use bson::Bson;

fn existing(ids: &[i32]) -> ExistingIds {
    ExistingIds::from_ids(ids.iter().map(|i| Bson::Int32(*i)).collect())
}

fn docs(ids: &[i32]) -> Vec<bson::Document> {
    ids.iter().map(|i| mock_doc_with_id(*i)).collect()
}

#[test]
fn default_policy_is_skip() {
    assert_eq!(ConflictPolicy::default(), ConflictPolicy::Skip);
}

#[test]
fn skip_inserts_only_new_documents() {
    let plan = plan_batch(ConflictPolicy::Skip, docs(&[1, 2]), &existing(&[1]));
    assert_eq!(
        plan.ops,
        vec![WriteOp::Insert {
            doc: mock_doc_with_id(2)
        }]
    );
    assert_eq!(plan.skipped, 1);
}

#[test]
fn overwrite_replaces_existing_and_inserts_nothing() {
    let plan = plan_batch(ConflictPolicy::Overwrite, docs(&[1, 2]), &existing(&[1]));
    assert_eq!(
        plan.ops,
        vec![WriteOp::Replace {
            id: Bson::Int32(1),
            doc: mock_doc_with_id(1)
        }]
    );
    assert_eq!(plan.skipped, 1);
}

#[test]
fn upsert_replaces_existing_and_inserts_new() {
    let plan = plan_batch(ConflictPolicy::Upsert, docs(&[1, 2]), &existing(&[1]));
    assert_eq!(
        plan.ops,
        vec![
            WriteOp::Replace {
                id: Bson::Int32(1),
                doc: mock_doc_with_id(1)
            },
            WriteOp::Insert {
                doc: mock_doc_with_id(2)
            },
        ]
    );
    assert_eq!(plan.skipped, 0);
}

#[test]
fn existing_ids_match_across_numeric_types() {
    let ids = ExistingIds::from_ids(vec![Bson::Int64(1)]);
    assert!(ids.contains(&Bson::Int32(1)));
    assert!(!ids.contains(&Bson::Int32(2)));
}

#[test]
fn skipped_counts_documents_without_an_op() {
    let plan = plan_batch(
        ConflictPolicy::Skip,
        docs(&[1, 2, 3]),
        &existing(&[1, 2, 3]),
    );
    assert!(plan.ops.is_empty());
    assert_eq!(plan.skipped, 3);
}
