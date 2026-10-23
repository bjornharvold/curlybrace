//! Read-only connections and compare apply: a read-only connection never receives a write,
//! whichever side of the compare it is on. The refusal is judged by the destination side of
//! each operation, before the job slot is taken, and a lock that lands mid-apply fails the
//! remaining writes as `ReadOnly` (nothing was written), not as an unknown outcome.

use super::apply_bulk_tests::bulk;
use super::apply_tests::{barrier, collect_apply, op, pair_reports, send_apply, within};
use super::closed_tests::Hooks;
use super::read_only_tests::{
    Rig, SRC, TGT, open, rig, rig_with_parked, run_to_completion, send_lock, send_open, side,
};
use super::tests::{collect_run, next_compare, ns, pair, same_pair, send_start};
use crate::compare::apply::{ApplyKind, ApplyOp, Direction, IdPolicy};
use crate::compare::diff::PathSegment;
use crate::compare::protocol::{BulkApply, CompareCommand, CompareEvent};
use crate::compare::results::Category;
use crate::error::CoreError;
use crate::runtime::Command;

/// Key 2 is `Different` in pair 1: it exists on both sides.
const DIFFERENT: i32 = 2;

fn to_source(kind: ApplyKind) -> ApplyOp {
    ApplyOp {
        direction: Direction::TargetToSource,
        ..op(1, DIFFERENT, kind)
    }
}

fn fields_to_source() -> ApplyOp {
    ApplyOp {
        fields: vec![vec![PathSegment::Field("v".into())]],
        ..to_source(ApplyKind::Fields)
    }
}

fn send(r: &Rig, run: u64, ops: Vec<ApplyOp>, bulk: Vec<BulkApply>) {
    let apply = CompareCommand::Apply {
        session: 1,
        run,
        ops,
        bulk,
        id_policy: IdPolicy::Keep,
    };
    r.tx.send(Command::Compare(apply)).unwrap();
}

async fn expect_read_only(r: &mut Rig) {
    match next_compare(&mut r.h).await {
        CompareEvent::Failed {
            session: 1,
            context: "compare_apply",
            error,
        } => assert_eq!(error, CoreError::ReadOnly),
        other => panic!("{other:?}"),
    }
}

/// The refused apply took no job slot: a valid apply onto the writable target still runs.
async fn assert_slot_free(r: &mut Rig) {
    send_apply(&r.tx, 9, vec![op(1, 1, ApplyKind::Insert)]);
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!(reports.len(), 1, "{events:?}");
    assert_eq!(reports[0].1.applied, 1, "{events:?}");
}

async fn read_only_source_rig() -> Rig {
    let mut r = rig();
    open(&mut r, &side(SRC, true), &side(TGT, false)).await;
    run_to_completion(&mut r).await;
    r
}

#[tokio::test]
async fn copy_to_a_read_only_source_is_refused_before_any_write() {
    let mut r = read_only_source_rig().await;
    send(&r, 5, vec![to_source(ApplyKind::Replace)], Vec::new());
    expect_read_only(&mut r).await;
    assert_eq!(r.source.writes(), 0);
    assert_slot_free(&mut r).await;
}

#[tokio::test]
async fn field_copy_to_a_read_only_source_is_refused_before_any_write() {
    let mut r = read_only_source_rig().await;
    send(&r, 5, vec![fields_to_source()], Vec::new());
    expect_read_only(&mut r).await;
    assert_eq!(r.source.writes(), 0);
    assert_slot_free(&mut r).await;
}

#[tokio::test]
async fn bulk_copy_to_a_read_only_source_is_refused_before_any_write() {
    let mut r = read_only_source_rig().await;
    let to_source = bulk(
        1,
        Category::Different,
        ApplyKind::Replace,
        Direction::TargetToSource,
    );
    send(&r, 5, Vec::new(), vec![to_source]);
    expect_read_only(&mut r).await;
    assert_eq!(r.source.writes(), 0);
    assert_slot_free(&mut r).await;
}

#[tokio::test]
async fn a_mixed_apply_with_one_op_to_a_read_only_source_writes_nothing() {
    let mut r = read_only_source_rig().await;
    let ops = vec![op(1, 1, ApplyKind::Insert), fields_to_source()];
    send(&r, 5, ops, Vec::new());
    expect_read_only(&mut r).await;
    assert_eq!((r.source.writes(), r.target.writes()), (0, 0));
    assert_slot_free(&mut r).await;
}

#[tokio::test]
async fn a_locked_target_still_allows_a_copy_to_the_writable_source() {
    let mut r = rig();
    let target = side(TGT, false);
    open(&mut r, &side(SRC, false), &target).await;
    run_to_completion(&mut r).await;
    send_lock(&r.tx, target.id);
    send_apply(&r.tx, 5, vec![to_source(ApplyKind::Replace)]);
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!(reports.len(), 1, "{events:?}");
    assert_eq!(reports[0].1.applied, 1, "{events:?}");
    assert_eq!((r.source.writes(), r.target.writes()), (1, 0));
}

#[tokio::test]
async fn a_source_locked_while_opening_refuses_a_copy_to_source() {
    let hooks = std::sync::Arc::new(Hooks::default());
    let mut r = rig_with_parked(SRC, &hooks);
    let source_cfg = side(SRC, false);
    send_open(&r.tx, &source_cfg, &side(TGT, false));
    send_lock(&r.tx, source_cfg.id);
    hooks.entered.notified().await;
    hooks.release.notify_one();
    assert_eq!(
        next_compare(&mut r.h).await,
        CompareEvent::Opened { session: 1 }
    );
    run_to_completion(&mut r).await;
    send(&r, 5, vec![to_source(ApplyKind::Replace)], Vec::new());
    expect_read_only(&mut r).await;
    assert_eq!(r.source.writes(), 0);
}

#[tokio::test]
async fn a_lock_during_an_apply_fails_the_remaining_writes_as_read_only() {
    let mut r = rig();
    let target = side(TGT, false);
    open(&mut r, &side(SRC, false), &target).await;
    send_start(
        &r.tx,
        1,
        vec![same_pair(1), pair(2, ns("d", "c2"), ns("d", "c2"))],
    );
    collect_run(&mut r.h).await;
    let gate = std::sync::Arc::new(super::apply_tests::Gate::default());
    r.target.park_next_bulk(&gate);
    send_apply(
        &r.tx,
        5,
        vec![op(1, 1, ApplyKind::Insert), op(2, 1, ApplyKind::Insert)],
    );
    within(gate.entered.notified()).await;
    send_lock(&r.tx, target.id);
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    let events = within(collect_apply(&mut r.h)).await;
    let reports = pair_reports(&events);
    assert_eq!(reports[0].1.applied, 1, "{events:?}");
    let second = &reports[1].1;
    assert_eq!(second.failed, 1, "{events:?}");
    assert_eq!(second.failures[0].error, CoreError::ReadOnly);
    // Only pair 1's bulk write reached the store; pair 2's was refused by the lock.
    assert_eq!(r.target.writes(), 1);
}
