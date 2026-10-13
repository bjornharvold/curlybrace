//! Field-level apply ops through the runtime: the up-front validation (shape, duplicates, bulk,
//! destination kinds) and the happy path with its events, direction, cancel and close.

use bson::{Bson, Document, doc};

use super::apply_bulk_tests::bulk;
use super::apply_tests::{
    ApplyStore, Rig, assert_silent, barrier, collect_apply, default_rig, op, pair_reports,
    park_target_bulk, rig, send_apply, source_store, target_store, within,
};
use super::tests::{collect_run, next_compare, ns, same_pair, send_start};
use crate::compare::apply::{ApplyKind, ApplyOp, Direction};
use crate::compare::diff::PathSegment;
use crate::compare::key::KeyTuple;
use crate::compare::protocol::{CompareCommand, CompareEvent};
use crate::compare::results::Category;
use crate::error::CoreError;
use crate::runtime::Command;
use crate::store::CollectionKind;
use crate::test_support::InMemoryStore;

const NOT_FIT: &str = "operation does not fit this category";

fn path(name: &str) -> Vec<PathSegment> {
    vec![PathSegment::Field(name.into())]
}

fn fields_op(pair: u32, id: i32, direction: Direction, names: &[&str]) -> ApplyOp {
    ApplyOp {
        direction,
        fields: names.iter().map(|n| path(n)).collect(),
        ..op(pair, id, ApplyKind::Fields)
    }
}

/// Documents that differ in `v` and `w` on both sides, in `d.c` and `d.c2`.
fn differing(side: &str) -> InMemoryStore {
    let docs = |prefix: &str| {
        vec![doc! { "_id": 1, "v": format!("{prefix}-v"), "w": format!("{prefix}-w") }]
    };
    InMemoryStore::new()
        .with_docs(&ns("d", "c"), docs(side))
        .with_docs(&ns("d", "c2"), docs(side))
}

async fn differing_rig() -> Rig {
    rig(
        ApplyStore::new(differing("s")),
        ApplyStore::new(differing("t")),
    )
    .await
}

async fn expect_refused(r: &mut Rig, text: &str) {
    match next_compare(&mut r.h).await {
        CompareEvent::Failed {
            context: "compare_apply",
            error: CoreError::Unsupported(got),
            ..
        } => assert_eq!(got, text),
        other => panic!("{other:?}"),
    }
}

async fn expect_not_writable(r: &mut Rig) {
    match next_compare(&mut r.h).await {
        CompareEvent::Failed { error, .. } => assert_eq!(error, CoreError::ApplyTargetNotWritable),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn fields_op_to_a_view_destination_is_refused_and_writes_nothing() {
    let target = ApplyStore::with_kinds(target_store(), vec![("c", CollectionKind::View)]);
    let mut r = rig(ApplyStore::new(source_store()), target).await;
    send_apply(
        &r.tx,
        5,
        vec![fields_op(1, 2, Direction::SourceToTarget, &["v"])],
    );
    expect_not_writable(&mut r).await;
    assert_eq!(r.target.writes(), 0);
    // The slot was freed: an immediate Start is accepted.
    send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
}

#[tokio::test]
async fn fields_op_to_a_timeseries_destination_is_refused() {
    let target = ApplyStore::with_kinds(target_store(), vec![("c", CollectionKind::Timeseries)]);
    let mut r = rig(ApplyStore::new(source_store()), target).await;
    send_apply(
        &r.tx,
        5,
        vec![fields_op(1, 2, Direction::SourceToTarget, &["v"])],
    );
    expect_not_writable(&mut r).await;
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn a_fields_op_checks_the_side_its_direction_writes() {
    // Target-to-source writes the SOURCE: a view there refuses, a view on the target does not.
    let source = ApplyStore::with_kinds(differing("s"), vec![("c", CollectionKind::View)]);
    let mut r = rig(source, ApplyStore::new(differing("t"))).await;
    let back = fields_op(1, 1, Direction::TargetToSource, &["v"]);
    send_apply(&r.tx, 5, vec![back]);
    expect_not_writable(&mut r).await;
    let forward = fields_op(1, 1, Direction::SourceToTarget, &["v"]);
    send_apply(&r.tx, 6, vec![forward]);
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1);
}

#[tokio::test]
async fn a_timeseries_destination_still_accepts_inserts() {
    let target = ApplyStore::with_kinds(target_store(), vec![("c", CollectionKind::Timeseries)]);
    let mut r = rig(ApplyStore::new(source_store()), target).await;
    send_apply(&r.tx, 5, vec![op(1, 1, ApplyKind::Insert)]);
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1);
}

#[tokio::test]
async fn fields_op_with_no_paths_is_refused() {
    let mut r = default_rig().await;
    send_apply(&r.tx, 5, vec![op(1, 2, ApplyKind::Fields)]);
    expect_refused(&mut r, NOT_FIT).await;
    assert_eq!(r.target.writes(), 0);
    // Refused before the slot was taken.
    send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
}

#[tokio::test]
async fn a_whole_document_op_with_paths_is_refused() {
    let mut r = default_rig().await;
    for (run, kind, id) in [
        (5, ApplyKind::Insert, 1),
        (6, ApplyKind::Replace, 2),
        (7, ApplyKind::Delete, 2),
    ] {
        let with_paths = ApplyOp {
            fields: vec![path("v")],
            ..op(1, id, kind)
        };
        send_apply(&r.tx, run, vec![with_paths]);
        expect_refused(&mut r, NOT_FIT).await;
    }
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn fields_and_replace_on_one_key_are_refused_as_duplicates() {
    let mut r = default_rig().await;
    let fields = fields_op(1, 2, Direction::SourceToTarget, &["v"]);
    send_apply(&r.tx, 5, vec![fields.clone(), op(1, 2, ApplyKind::Replace)]);
    expect_refused(&mut r, "duplicate operation").await;
    let wide = ApplyOp {
        key: KeyTuple(vec![Bson::Int64(2)]),
        ..op(1, 2, ApplyKind::Replace)
    };
    send_apply(&r.tx, 6, vec![fields.clone(), wide]);
    expect_refused(&mut r, "duplicate operation").await;
    send_apply(&r.tx, 7, vec![fields.clone(), fields]);
    expect_refused(&mut r, "duplicate operation").await;
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn a_bulk_item_of_kind_fields_is_refused() {
    let mut r = default_rig().await;
    for category in [Category::OnlyInSource, Category::Different] {
        let item = bulk(1, category, ApplyKind::Fields, Direction::SourceToTarget);
        let apply = CompareCommand::Apply {
            session: 1,
            run: 5,
            ops: vec![],
            bulk: vec![item],
            id_policy: crate::compare::apply::IdPolicy::Keep,
        };
        r.tx.send(Command::Compare(apply)).unwrap();
        expect_refused(&mut r, NOT_FIT).await;
    }
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn fields_ops_apply_through_the_runtime_with_events_in_order() {
    let mut r = differing_rig().await;
    send_apply(
        &r.tx,
        5,
        vec![fields_op(1, 1, Direction::SourceToTarget, &["v"])],
    );
    let events = within(collect_apply(&mut r.h)).await;
    let tags: Vec<&str> = events
        .iter()
        .map(|e| match e {
            CompareEvent::ApplyProgress { .. } => "progress",
            CompareEvent::ApplyPairDone { .. } => "pair",
            CompareEvent::ApplyDone { .. } => "done",
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(tags, ["progress", "pair", "done"]);
    assert_eq!(pair_reports(&events)[0].1.applied, 1);
    let expected = doc! { "_id": 1, "v": "s-v", "w": "t-w" };
    assert_eq!(r.target.docs(&ns("d", "c")).await, vec![expected]);
    assert_eq!(
        r.source.docs(&ns("d", "c")).await,
        vec![doc! { "_id": 1, "v": "s-v", "w": "s-w" }]
    );
}

fn by_id(docs: Vec<Document>) -> Vec<Document> {
    docs
}

#[tokio::test]
async fn fields_ops_in_both_directions_in_one_apply() {
    let mut r = differing_rig().await;
    let ops = vec![
        fields_op(1, 1, Direction::SourceToTarget, &["v"]),
        fields_op(2, 1, Direction::TargetToSource, &["w"]),
    ];
    send_apply(&r.tx, 5, ops);
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(|(_, rep)| rep.applied == 1));
    let c_target = by_id(r.target.docs(&ns("d", "c")).await);
    assert_eq!(c_target, vec![doc! { "_id": 1, "v": "s-v", "w": "t-w" }]);
    let c2_source = by_id(r.source.docs(&ns("d", "c2")).await);
    assert_eq!(c2_source, vec![doc! { "_id": 1, "v": "s-v", "w": "t-w" }]);
    // The sides not written stay as they were.
    let c_source = r.source.docs(&ns("d", "c")).await;
    assert_eq!(c_source, vec![doc! { "_id": 1, "v": "s-v", "w": "s-w" }]);
    let c2_target = r.target.docs(&ns("d", "c2")).await;
    assert_eq!(c2_target, vec![doc! { "_id": 1, "v": "t-v", "w": "t-w" }]);
}

#[tokio::test]
async fn cancel_mid_apply_with_a_fields_op_reports_not_run_and_frees_the_slot() {
    let mut r = differing_rig().await;
    let gate = park_target_bulk(&r);
    let ops = vec![
        fields_op(1, 1, Direction::SourceToTarget, &["v"]),
        fields_op(2, 1, Direction::SourceToTarget, &["v"]),
    ];
    send_apply(&r.tx, 5, ops);
    within(gate.entered.notified()).await;
    let cancel = CompareCommand::Cancel { session: 1, run: 5 };
    r.tx.send(Command::Compare(cancel)).unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    let reports = pair_reports(&within(collect_apply(&mut r.h)).await);
    assert_eq!(reports[0].1.applied, 1, "the batch in flight finishes");
    assert_eq!(reports[1].1.not_run, 1);
    let untouched = doc! { "_id": 1, "v": "t-v", "w": "t-w" };
    assert_eq!(r.target.docs(&ns("d", "c2")).await, vec![untouched]);
    send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
}

#[tokio::test]
async fn closing_the_session_mid_fields_apply_emits_nothing_more() {
    let mut r = differing_rig().await;
    let gate = park_target_bulk(&r);
    let ops = vec![
        fields_op(1, 1, Direction::SourceToTarget, &["v"]),
        fields_op(2, 1, Direction::SourceToTarget, &["v"]),
    ];
    send_apply(&r.tx, 5, ops);
    within(gate.entered.notified()).await;
    r.tx.send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    assert_silent(&mut r.h).await;
    let untouched = doc! { "_id": 1, "v": "t-v", "w": "t-w" };
    assert_eq!(r.target.docs(&ns("d", "c2")).await, vec![untouched]);
}
