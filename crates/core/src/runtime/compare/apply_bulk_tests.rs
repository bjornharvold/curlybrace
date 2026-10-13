//! Category-wide apply items: the runtime expands each `BulkApply` into one operation per stored
//! row of its category before validating, so the engine and the report are unchanged.

use bson::{Document, doc};

use super::apply_tests::{
    ApplyStore, Rig, collect_apply, default_rig, op, pair_reports, rig, source_store, target_store,
    within,
};
use super::tests::ns;
use crate::compare::apply::{ApplyKind, Direction, IdPolicy};
use crate::compare::protocol::{BulkApply, CompareCommand, CompareEvent};
use crate::compare::results::Category;
use crate::error::CoreError;
use crate::runtime::Command;
use crate::store::CollectionKind;
use crate::test_support::InMemoryStore;

fn bulk(pair: u32, category: Category, kind: ApplyKind, direction: Direction) -> BulkApply {
    BulkApply {
        pair,
        category,
        kind,
        direction,
    }
}

fn insert_all_in_source(pair: u32) -> BulkApply {
    bulk(
        pair,
        Category::OnlyInSource,
        ApplyKind::Insert,
        Direction::SourceToTarget,
    )
}

fn send_bulk(r: &Rig, run: u64, ops: Vec<crate::compare::apply::ApplyOp>, bulk: Vec<BulkApply>) {
    let apply = CompareCommand::Apply {
        session: 1,
        run,
        ops,
        bulk,
        id_policy: IdPolicy::Keep,
    };
    r.tx.send(Command::Compare(apply)).unwrap();
}

async fn expect_unsupported(r: &mut Rig, text: &str) {
    match super::tests::next_compare(&mut r.h).await {
        CompareEvent::Failed {
            context: "compare_apply",
            error: CoreError::Unsupported(got),
            ..
        } => assert_eq!(got, text),
        other => panic!("{other:?}"),
    }
}

fn numbered(range: std::ops::Range<i32>) -> Vec<Document> {
    range.map(|i| doc! { "_id": i, "v": i }).collect()
}

#[tokio::test]
async fn a_bulk_item_applies_every_row_of_the_category() {
    let source = InMemoryStore::new().with_docs(&ns("d", "c"), numbered(0..50));
    let target = InMemoryStore::new();
    let mut r = rig(ApplyStore::new(source), ApplyStore::new(target)).await;
    send_bulk(&r, 5, vec![], vec![insert_all_in_source(1)]);
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!(reports.len(), 1);
    assert_eq!((reports[0].1.applied, reports[0].1.failed), (50, 0));
    assert_eq!(r.target.docs(&ns("d", "c")).await.len(), 50);
}

#[tokio::test]
async fn a_bulk_item_that_does_not_fit_the_category_is_refused_and_writes_nothing() {
    let mut r = default_rig().await;
    let replace = bulk(
        1,
        Category::OnlyInSource,
        ApplyKind::Replace,
        Direction::SourceToTarget,
    );
    send_bulk(&r, 5, vec![], vec![replace]);
    expect_unsupported(&mut r, "operation does not fit this category").await;
    let delete = bulk(
        1,
        Category::OnlyInSource,
        ApplyKind::Delete,
        Direction::SourceToTarget,
    );
    send_bulk(&r, 6, vec![], vec![delete]);
    expect_unsupported(&mut r, "operation does not fit this category").await;
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn a_bulk_item_for_an_unknown_pair_is_refused() {
    let mut r = default_rig().await;
    send_bulk(&r, 5, vec![], vec![insert_all_in_source(99)]);
    expect_unsupported(&mut r, "no results for this pair").await;
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn a_bulk_item_and_an_explicit_op_on_the_same_key_are_refused_as_duplicates() {
    let mut r = default_rig().await;
    let explicit = vec![op(1, 1, ApplyKind::Insert)];
    send_bulk(&r, 5, explicit, vec![insert_all_in_source(1)]);
    expect_unsupported(&mut r, "duplicate operation").await;
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn two_bulk_items_for_one_category_are_refused() {
    let mut r = default_rig().await;
    let both = vec![insert_all_in_source(1), insert_all_in_source(1)];
    send_bulk(&r, 5, vec![], both);
    expect_unsupported(&mut r, "duplicate operation").await;
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn bulk_and_explicit_ops_for_different_categories_apply_together() {
    // Pair 1: _id 1 only in source, _id 2 differs.
    let mut r = default_rig().await;
    send_bulk(
        &r,
        5,
        vec![op(1, 2, ApplyKind::Replace)],
        vec![insert_all_in_source(1)],
    );
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!((reports[0].1.applied, reports[0].1.failed), (2, 0));
    assert_eq!(
        r.target.docs(&ns("d", "c")).await,
        vec![
            doc! { "_id": 1, "v": "SENTINEL-DOCUMENT-VALUE" },
            doc! { "_id": 2, "v": "new" }
        ]
    );
}

#[tokio::test]
async fn an_empty_category_expands_to_nothing_and_an_otherwise_empty_request_is_refused() {
    let mut r = default_rig().await;
    let empty = bulk(
        1,
        Category::OnlyInTarget,
        ApplyKind::Delete,
        Direction::SourceToTarget,
    );
    // Pair 1 has no only-in-target rows; the request would write nothing.
    send_bulk(&r, 5, vec![], vec![empty.clone()]);
    expect_unsupported(&mut r, "nothing to apply").await;
    // Fit is judged even when nothing is stored.
    let misfit = bulk(
        1,
        Category::OnlyInTarget,
        ApplyKind::Insert,
        Direction::SourceToTarget,
    );
    send_bulk(&r, 7, vec![], vec![misfit]);
    expect_unsupported(&mut r, "operation does not fit this category").await;
    // The same empty category twice is a repeat even though it expands to nothing.
    send_bulk(&r, 8, vec![], vec![empty.clone(), empty.clone()]);
    expect_unsupported(&mut r, "duplicate operation").await;
    // An explicit op alongside still applies.
    send_bulk(&r, 6, vec![op(1, 1, ApplyKind::Insert)], vec![empty]);
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1);
}

#[tokio::test]
async fn bulk_expansion_keeps_the_view_and_timeseries_destination_rules() {
    let target = ApplyStore::with_kinds(
        InMemoryStore::new().with_docs(&ns("d", "c3"), numbered(0..1)),
        vec![("c", CollectionKind::View)],
    );
    let source = ApplyStore::new(InMemoryStore::new().with_docs(&ns("d", "c"), numbered(0..3)));
    let mut r = rig(source, target).await;
    send_bulk(&r, 5, vec![], vec![insert_all_in_source(1)]);
    match super::tests::next_compare(&mut r.h).await {
        CompareEvent::Failed {
            context: "compare_apply",
            error,
            ..
        } => assert_eq!(error, CoreError::ApplyTargetNotWritable),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn exactly_the_six_fitting_combinations_are_accepted() {
    use ApplyKind::{Delete, Insert, Replace};
    use Category::{Different, OnlyInSource, OnlyInTarget};
    use Direction::{SourceToTarget as ST, TargetToSource as TS};
    let allowed = [
        (OnlyInSource, Insert, ST),
        (OnlyInSource, Delete, TS),
        (OnlyInTarget, Delete, ST),
        (OnlyInTarget, Insert, TS),
        (Different, Replace, ST),
        (Different, Replace, TS),
    ];
    let mut r = default_rig().await;
    let mut run = 10;
    for category in [OnlyInSource, OnlyInTarget, Different] {
        for kind in [Insert, Replace, Delete] {
            for direction in [ST, TS] {
                if allowed.contains(&(category, kind, direction)) {
                    continue;
                }
                run += 1;
                send_bulk(&r, run, vec![], vec![bulk(1, category, kind, direction)]);
                expect_unsupported(&mut r, "operation does not fit this category").await;
            }
        }
    }
    assert_eq!(run, 22, "12 refused combinations");
    assert_eq!(r.source.writes() + r.target.writes(), 0);
}

#[tokio::test]
async fn a_bulk_replace_into_a_timeseries_destination_is_refused_but_an_insert_is_allowed() {
    let target = ApplyStore::with_kinds(target_store(), vec![("c", CollectionKind::Timeseries)]);
    let mut r = rig(ApplyStore::new(source_store()), target).await;
    let replace = bulk(
        1,
        Category::Different,
        ApplyKind::Replace,
        Direction::SourceToTarget,
    );
    send_bulk(&r, 5, vec![], vec![replace]);
    match super::tests::next_compare(&mut r.h).await {
        CompareEvent::Failed { error, .. } => {
            assert_eq!(error, CoreError::ApplyTargetNotWritable)
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(r.target.writes(), 0);
    send_bulk(&r, 6, vec![], vec![insert_all_in_source(1)]);
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1);
}
