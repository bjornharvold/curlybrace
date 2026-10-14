//! Tests for export planning: real `run_pair` results over in-memory stores, a store that
//! refuses every write for the plan, and a recording store for the apply it must equal.

use super::*;

#[path = "plan_probe_tests.rs"]
mod probe;
use crate::compare::apply::{APPLY_BATCH, ApplyKind, ApplyReport, Direction, IdPolicy, SkipReason};
use crate::compare::apply_engine::apply_pair;
use crate::compare::diff::PathSegment;
use crate::compare::engine::{EngineOutcome, PairInput, run_pair};
use crate::compare::key::{KeySpec, KeyTuple};
use crate::compare::protocol::{PairSpec, Side};
use crate::compare::results::{Budget, Category, Limits, ResultSet};
use crate::compare::rules::{CompareRules, FieldScope};
use crate::store::{DocumentStore, Namespace, WriteOp};
use bson::{Bson, Document, doc};
use probe::Probe;
use std::sync::Arc;

use ApplyKind::{Delete, Fields, Insert, Replace};
use Direction::{SourceToTarget as ST, TargetToSource as TS};

const PAIR: u32 = 1;

fn src_ns() -> Namespace {
    Namespace::new("db", "a")
}

fn tgt_ns() -> Namespace {
    Namespace::new("db", "b")
}

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn op(key: KeyTuple, kind: ApplyKind, direction: Direction) -> ApplyOp {
    ApplyOp {
        pair: PAIR,
        key,
        kind,
        direction,
        fields: Vec::new(),
    }
}

fn fields_op(n: i32, direction: Direction, paths: &[&[&str]]) -> ApplyOp {
    ApplyOp {
        pair: PAIR,
        key: k(n),
        kind: Fields,
        direction,
        fields: paths
            .iter()
            .map(|p| p.iter().map(|s| PathSegment::Field((*s).into())).collect())
            .collect(),
    }
}

struct Rig {
    spec: PairSpec,
    results: ResultSet,
    source: Arc<Probe>,
    target: Arc<Probe>,
}

impl Rig {
    async fn new(
        source: Vec<Document>,
        target: Vec<Document>,
        rules: CompareRules,
        forbid_writes: bool,
    ) -> Self {
        let source = Probe::new(&src_ns(), source, forbid_writes);
        let target = Probe::new(&tgt_ns(), target, forbid_writes);
        let spec = PairSpec {
            id: PAIR,
            source: src_ns(),
            target: tgt_ns(),
            source_filter: doc! {},
            target_filter: doc! {},
            key: KeySpec::id(),
            rules,
        };
        let input = PairInput {
            source: source.clone(),
            source_ns: spec.source.clone(),
            source_filter: doc! {},
            target: target.clone(),
            target_ns: spec.target.clone(),
            target_filter: doc! {},
            key: spec.key.clone(),
            rules: spec.rules.clone(),
            budget: Budget::new(Limits::default()),
        };
        let (outcome, _, results) = run_pair(input, &CancelFlag::default(), &mut |_| {})
            .await
            .unwrap();
        assert_eq!(outcome, EngineOutcome::Completed);
        Self {
            spec,
            results,
            source,
            target,
        }
    }

    async fn plain(source: Vec<Document>, target: Vec<Document>, forbid: bool) -> Self {
        Self::new(source, target, rules(), forbid).await
    }

    fn input<'a>(&'a self, ops: &'a [ApplyOp], policy: IdPolicy) -> ApplyInput<'a> {
        ApplyInput {
            spec: &self.spec,
            results: &self.results,
            source: self.source.clone(),
            target: self.target.clone(),
            ops,
            policy,
        }
    }
}

fn rules() -> CompareRules {
    CompareRules::for_key(&KeySpec::id())
}

#[derive(Default)]
struct Out {
    items: Vec<ExportItem>,
    counts: ExportPlanCounts,
    batches: Vec<usize>,
    progress: Vec<u64>,
}

async fn plan_with(
    rig: &Rig,
    ops: &[ApplyOp],
    policy: IdPolicy,
    cancel: &CancelFlag,
    cancel_after_first_batch: bool,
) -> Out {
    let (mut items, mut batches, mut progress) = (Vec::new(), Vec::new(), Vec::new());
    let counts = plan_pair_export(
        rig.input(ops, policy),
        cancel,
        &mut |batch| {
            batches.push(batch.len());
            items.extend(batch);
            Ok(())
        },
        &mut |done| {
            progress.push(done);
            if cancel_after_first_batch {
                cancel.cancel();
            }
        },
    )
    .await
    .unwrap();
    Out {
        items,
        counts,
        batches,
        progress,
    }
}

async fn plan(rig: &Rig, ops: &[ApplyOp]) -> Out {
    plan_with(rig, ops, IdPolicy::Keep, &CancelFlag::default(), false).await
}

fn sum(c: &ExportPlanCounts) -> u64 {
    c.written
        + c.skipped_stale
        + c.skipped_not_planned
        + c.skipped_unsupported
        + c.failed
        + c.not_run
}

/// The writes of a plan that an apply would send to (source, target).
fn writes_by_side(items: &[ExportItem]) -> (Vec<WriteOp>, Vec<WriteOp>) {
    let (mut source, mut target) = (Vec::new(), Vec::new());
    for item in items {
        if let ExportItem::Write { write, .. } = item {
            match item.destination() {
                Some(Side::Target) => target.push(write.clone()),
                Some(Side::Source) => source.push(write.clone()),
                None => {}
            }
        }
    }
    (source, target)
}

/// Plans `ops` on one rig (writes forbidden) and applies them on an identical one; the plan must
/// hold exactly the writes the apply performed, and the counts must match the apply report.
async fn assert_plan_equals_apply(
    source: Vec<Document>,
    target: Vec<Document>,
    rules: CompareRules,
    ops: &[ApplyOp],
    policy: IdPolicy,
) -> Out {
    let planning = Rig::new(source.clone(), target.clone(), rules.clone(), true).await;
    let out = plan_with(&planning, ops, policy, &CancelFlag::default(), false).await;
    let applying = Rig::new(source, target, rules, false).await;
    let report: ApplyReport = apply_pair(
        applying.input(ops, policy),
        &CancelFlag::default(),
        &mut |_| {},
    )
    .await
    .unwrap();
    let (to_source, to_target) = writes_by_side(&out.items);
    assert_eq!(to_source, applying.source.recorded());
    assert_eq!(to_target, applying.target.recorded());
    assert_eq!(out.counts.written, report.applied);
    assert_eq!(out.counts.skipped_stale, report.stale);
    assert_eq!(out.counts.skipped_not_planned, report.not_planned);
    assert_eq!(out.counts.failed, report.failed);
    assert_eq!(sum(&out.counts), ops.len() as u64);
    out
}

fn skipped(out: &Out, reason: ExportSkip) -> usize {
    out.items
        .iter()
        .filter(|i| matches!(i, ExportItem::Skipped { reason: r, .. } if *r == reason))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_insert_plan_equals_the_apply() {
    let out = assert_plan_equals_apply(
        vec![doc! {"_id": 1, "v": 1}, doc! {"_id": 2, "v": 2}],
        vec![],
        rules(),
        &[op(k(1), Insert, ST), op(k(2), Insert, ST)],
        IdPolicy::Keep,
    )
    .await;
    assert_eq!(out.counts.written, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replace_plan_equals_the_apply_in_both_directions() {
    let source = vec![doc! {"_id": 1, "v": 1}, doc! {"_id": 2, "v": 5}];
    let target = vec![doc! {"_id": 1, "v": 2}, doc! {"_id": 2, "v": 6}];
    let out = assert_plan_equals_apply(
        source,
        target,
        rules(),
        &[op(k(1), Replace, ST), op(k(2), Replace, TS)],
        IdPolicy::Keep,
    )
    .await;
    let (to_source, to_target) = writes_by_side(&out.items);
    assert_eq!((to_source.len(), to_target.len()), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delete_plan_equals_the_apply_in_both_directions() {
    let out = assert_plan_equals_apply(
        vec![doc! {"_id": 1}],
        vec![doc! {"_id": 2}],
        rules(),
        &[op(k(1), Delete, TS), op(k(2), Delete, ST)],
        IdPolicy::Keep,
    )
    .await;
    assert_eq!(out.counts.written, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fields_plan_equals_the_apply() {
    let out = assert_plan_equals_apply(
        vec![doc! {"_id": 1, "a": 1, "b": 1, "c": {"x": 1}}],
        vec![doc! {"_id": 1, "a": 2, "b": 1, "gone": 1, "c": {"x": 2}}],
        rules(),
        &[fields_op(1, ST, &[&["a"], &["gone"], &["c", "x"]])],
        IdPolicy::Keep,
    )
    .await;
    assert!(matches!(
        out.items[0],
        ExportItem::Write {
            write: WriteOp::UpdateFields { .. },
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_batch_equals_the_apply_and_counts_add_up() {
    let source = vec![
        doc! {"_id": 1, "v": 1},
        doc! {"_id": 3, "v": 1},
        doc! {"_id": 4, "v": 1},
        doc! {"_id": 5, "a": 1},
    ];
    let target = vec![
        doc! {"_id": 2, "v": 1},
        doc! {"_id": 3, "v": 2},
        doc! {"_id": 4, "v": 2},
        doc! {"_id": 5, "a": 2},
    ];
    let ops = [
        op(k(1), Insert, ST),
        op(k(2), Insert, TS),
        op(k(3), Replace, ST),
        op(k(4), Replace, TS),
        op(k(2), Delete, ST),
        op(k(1), Delete, TS),
        fields_op(5, TS, &[&["a"]]),
        op(k(3), Insert, ST),
    ];
    let out = assert_plan_equals_apply(source, target, rules(), &ops, IdPolicy::Keep).await;
    assert_eq!(out.items.len(), ops.len());
    // Item i belongs to op i.
    for (item, expected) in out.items.iter().zip(&ops) {
        let (ExportItem::Write { op, .. }
        | ExportItem::Skipped { op, .. }
        | ExportItem::Failed { op, .. }) = item;
        assert_eq!(op, expected);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn generate_drops_the_id_on_inserts_like_apply() {
    let out = assert_plan_equals_apply(
        vec![doc! {"_id": 1, "v": 1}],
        vec![],
        rules(),
        &[op(k(1), Insert, ST)],
        IdPolicy::Generate,
    )
    .await;
    let ExportItem::Write { write, .. } = &out.items[0] else {
        panic!("{:?}", out.items);
    };
    assert_eq!(write, &WriteOp::Insert { doc: doc! {"v": 1} });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_changed_on_either_side_is_stale() {
    let rig = Rig::plain(
        vec![doc! {"_id": 1, "v": 1}, doc! {"_id": 2, "v": 1}],
        vec![doc! {"_id": 1, "v": 2}, doc! {"_id": 2, "v": 2}],
        true,
    )
    .await;
    rig.source
        .inner
        .replace(&src_ns(), &Bson::Int32(1), doc! {"_id": 1, "v": 9})
        .await
        .unwrap();
    rig.target
        .inner
        .replace(&tgt_ns(), &Bson::Int32(2), doc! {"_id": 2, "v": 9})
        .await
        .unwrap();
    let out = plan(&rig, &[op(k(1), Replace, ST), op(k(2), Replace, ST)]).await;
    assert_eq!(skipped(&out, ExportSkip::Stale), 2);
    assert_eq!((out.counts.skipped_stale, sum(&out.counts)), (2, 2));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_that_appeared_or_vanished_is_stale() {
    let rig = Rig::plain(vec![doc! {"_id": 1}], vec![doc! {"_id": 2}], true).await;
    rig.target
        .inner
        .insert(&tgt_ns(), doc! {"_id": 1})
        .await
        .unwrap();
    rig.target
        .inner
        .delete(&tgt_ns(), &Bson::Int32(2))
        .await
        .unwrap();
    let out = plan(&rig, &[op(k(1), Insert, ST), op(k(2), Delete, ST)]).await;
    assert_eq!(out.counts.skipped_stale, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn kinds_that_do_not_fit_the_row_are_not_planned() {
    let rig = Rig::plain(
        vec![doc! {"_id": 1}, doc! {"_id": 3, "v": 1}],
        vec![doc! {"_id": 2}, doc! {"_id": 3, "v": 2}],
        true,
    )
    .await;
    let ops = [
        op(k(1), Delete, ST),
        op(k(1), Replace, ST),
        op(k(2), Insert, ST),
        op(k(3), Insert, ST),
        op(k(3), Delete, TS),
        op(k(9), Insert, ST),
    ];
    let out = plan(&rig, &ops).await;
    // The last op has no recorded row: apply reports it as a failure, so does the plan.
    assert_eq!((out.counts.skipped_not_planned, out.counts.failed), (5, 1));
    assert_eq!(sum(&out.counts), 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_key_is_planned_once_exactly_as_in_apply() {
    let out = assert_plan_equals_apply(
        vec![doc! {"_id": 1, "v": 1}],
        vec![doc! {"_id": 1, "v": 2}],
        rules(),
        &[
            op(k(1), Replace, ST),
            op(k(1), Replace, ST),
            op(k(1), Replace, TS),
        ],
        IdPolicy::Keep,
    )
    .await;
    assert_eq!((out.counts.written, out.counts.skipped_not_planned), (1, 2));
}

#[tokio::test(flavor = "multi_thread")]
async fn unaddressable_and_scope_unsafe_field_paths_behave_as_in_apply() {
    let out = assert_plan_equals_apply(
        vec![doc! {"_id": 1, "a.b": 1, "$x": 1, "out": 1, "meta": {"owner": "o"}}],
        vec![doc! {"_id": 1, "a.b": 2, "$x": 2, "out": 2, "meta": {"owner": "x"}}],
        CompareRules {
            field_scope: FieldScope::Only(vec!["meta.owner".into()]),
            ..rules()
        },
        &[
            fields_op(1, ST, &[&["a.b"]]),
            fields_op(1, ST, &[&["$x"]]),
            fields_op(1, ST, &[&["out"]]),
            fields_op(1, ST, &[&["meta", "owner"]]),
        ],
        IdPolicy::Keep,
    )
    .await;
    // Same key four times: only the first op runs; the rest are repeats.
    assert_eq!(out.counts.skipped_not_planned, 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn each_unaddressable_field_is_refused_on_its_own() {
    for path in [&["a.b"][..], &["$x"][..], &["out"][..]] {
        let out = assert_plan_equals_apply(
            vec![doc! {"_id": 1, "a.b": 1, "$x": 1, "out": 1, "meta": {"owner": "o"}}],
            vec![doc! {"_id": 1, "a.b": 2, "$x": 2, "out": 2, "meta": {"owner": "x"}}],
            CompareRules {
                field_scope: FieldScope::Only(vec!["meta.owner".into(), "a.b".into(), "$x".into()]),
                ..rules()
            },
            &[fields_op(1, ST, &[path])],
            IdPolicy::Keep,
        )
        .await;
        assert_eq!(out.counts.skipped_not_planned, 1, "{path:?}");
    }
}

fn db_pointer() -> Bson {
    let mut raw: Vec<u8> = vec![26, 0, 0, 0, 0x0C, b'p', 0, 2, 0, 0, 0, b'c', 0];
    raw.extend([0u8; 12]);
    raw.push(0);
    Document::from_reader(raw.as_slice())
        .unwrap()
        .get("p")
        .unwrap()
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn values_the_shell_cannot_represent_are_skipped_as_unsupported() {
    let bad_regex = Bson::RegularExpression(bson::Regex {
        pattern: "a".into(),
        options: "g".into(),
    });
    let rig = Rig::plain(
        vec![
            doc! {"_id": 1, "u": Bson::Undefined},
            doc! {"_id": 2, "p": db_pointer()},
            doc! {"_id": 3, "r": bad_regex},
            doc! {"_id": 4, "ok": 1},
        ],
        vec![],
        true,
    )
    .await;
    let ops: Vec<ApplyOp> = (1..=4).map(|n| op(k(n), Insert, ST)).collect();
    let out = plan(&rig, &ops).await;
    assert_eq!(skipped(&out, ExportSkip::Unsupported), 3);
    assert_eq!((out.counts.written, out.counts.skipped_unsupported), (1, 3));
    assert_eq!(sum(&out.counts), 4);
    // The unsupported ones never reach a writer as a write.
    let writes: Vec<_> = writes_by_side(&out.items).1;
    assert_eq!(
        writes,
        vec![WriteOp::Insert {
            doc: doc! {"_id": 4, "ok": 1}
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsupported_replacement_is_skipped_but_a_delete_of_it_is_not() {
    let rig = Rig::plain(
        vec![doc! {"_id": 1, "u": Bson::Undefined}],
        vec![
            doc! {"_id": 1, "v": 1},
            doc! {"_id": 2, "u": Bson::Undefined},
        ],
        true,
    )
    .await;
    let out = plan(&rig, &[op(k(1), Replace, ST), op(k(2), Delete, ST)]).await;
    assert_eq!((out.counts.skipped_unsupported, out.counts.written), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn items_carry_the_recorded_fingerprints() {
    let rig = Rig::plain(
        vec![doc! {"_id": 1, "v": 1}, doc! {"_id": 2}],
        vec![doc! {"_id": 1, "v": 2}, doc! {"_id": 3}],
        true,
    )
    .await;
    let ops = [
        op(k(1), Replace, ST),
        op(k(2), Insert, ST),
        op(k(3), Delete, ST),
        op(k(2), Delete, ST),
    ];
    let out = plan(&rig, &ops).await;
    for item in &out.items {
        let (ExportItem::Write {
            op,
            source_fp,
            target_fp,
            ..
        }
        | ExportItem::Skipped {
            op,
            source_fp,
            target_fp,
            ..
        }) = item
        else {
            panic!("{item:?}");
        };
        let entry = [
            Category::Different,
            Category::OnlyInSource,
            Category::OnlyInTarget,
        ]
        .iter()
        .find_map(|c| rig.results.find(*c, &op.key))
        .unwrap();
        assert_eq!((*source_fp, *target_fp), (entry.source, entry.target));
    }
    let ExportItem::Write {
        source_fp,
        target_fp,
        ..
    } = &out.items[1]
    else {
        panic!()
    };
    assert!(source_fp.is_some() && target_fp.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_error_fails_that_op_and_the_batch_goes_on() {
    let rig = Rig::plain(
        vec![doc! {"_id": 1}, doc! {"_id": 2}, doc! {"_id": 3}],
        vec![],
        true,
    )
    .await;
    *rig.source.fail_id.lock().unwrap() = Some(2);
    let ops: Vec<ApplyOp> = (1..=3).map(|n| op(k(n), Insert, ST)).collect();
    let out = plan(&rig, &ops).await;
    assert_eq!((out.counts.written, out.counts.failed), (2, 1));
    assert!(matches!(out.items[1], ExportItem::Failed { .. }));
    assert_eq!(sum(&out.counts), 3);
}

async fn many_inserts(n: i32) -> (Rig, Vec<ApplyOp>) {
    let docs: Vec<Document> = (1..=n).map(|i| doc! {"_id": i}).collect();
    let rig = Rig::plain(docs, vec![], true).await;
    let ops = (1..=n).map(|i| op(k(i), Insert, ST)).collect();
    (rig, ops)
}

#[tokio::test(flavor = "multi_thread")]
async fn batches_follow_the_apply_batch_size() {
    assert_eq!(APPLY_BATCH, 500);
    for (n, batches) in [
        (0, vec![]),
        (1, vec![1]),
        (500, vec![500]),
        (501, vec![500, 1]),
    ] {
        let (rig, ops) = many_inserts(n).await;
        let out = plan(&rig, &ops).await;
        assert_eq!(out.batches, batches, "{n} ops");
        let progress: Vec<u64> = batches
            .iter()
            .scan(0u64, |done, b| {
                *done += *b as u64;
                Some(*done)
            })
            .collect();
        assert_eq!(out.progress, progress, "{n} ops");
        assert_eq!((out.counts.written, sum(&out.counts)), (n as u64, n as u64));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_between_batches_counts_the_rest_as_not_run() {
    let (rig, ops) = many_inserts(1001).await;
    let cancel = CancelFlag::default();
    let out = plan_with(&rig, &ops, IdPolicy::Keep, &cancel, true).await;
    assert_eq!(out.batches, vec![500]);
    assert_eq!((out.counts.written, out.counts.not_run), (500, 501));
    assert_eq!(sum(&out.counts), 1001);
    assert_eq!(out.progress, vec![500]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_before_the_start_runs_nothing() {
    let (rig, ops) = many_inserts(3).await;
    let cancel = CancelFlag::default();
    cancel.cancel();
    let out = plan_with(&rig, &ops, IdPolicy::Keep, &cancel, false).await;
    assert_eq!(out.counts.not_run, 3);
    assert!(out.items.is_empty() && out.batches.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_sink_stops_the_plan_with_its_error() {
    let (rig, ops) = many_inserts(501).await;
    let (mut calls, mut reported) = (0, 0);
    let error = crate::error::CoreError::Database("sink".into());
    let result = plan_pair_export(
        rig.input(&ops, IdPolicy::Keep),
        &CancelFlag::default(),
        &mut |_| {
            calls += 1;
            Err(error.clone())
        },
        &mut |_| reported += 1,
    )
    .await;
    assert_eq!(result.unwrap_err(), error);
    assert_eq!((calls, reported), (1, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_repeated_in_a_later_batch_is_not_planned_as_in_apply() {
    let docs: Vec<Document> = (1..=500).map(|i| doc! {"_id": i}).collect();
    let mut ops: Vec<ApplyOp> = (1..=500).map(|i| op(k(i), Insert, ST)).collect();
    ops.push(op(k(1), Insert, ST));
    let out = assert_plan_equals_apply(docs, vec![], rules(), &ops, IdPolicy::Keep).await;
    assert_eq!(out.batches, vec![500, 1]);
    assert!(matches!(
        out.items[500],
        ExportItem::Skipped {
            reason: ExportSkip::NotPlanned,
            ..
        }
    ));
    assert_eq!(
        (out.counts.written, out.counts.skipped_not_planned),
        (500, 1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fields_value_the_shell_cannot_represent_is_unsupported_though_apply_writes_it() {
    let source = vec![doc! {"_id": 1, "a": Bson::Undefined}];
    let target = vec![doc! {"_id": 1, "a": 1}];
    let ops = [fields_op(1, ST, &[&["a"]])];
    let planning = Rig::plain(source.clone(), target.clone(), true).await;
    let out = plan(&planning, &ops).await;
    assert_eq!(skipped(&out, ExportSkip::Unsupported), 1);
    assert_eq!(out.counts.skipped_unsupported, 1);
    let applying = Rig::plain(source, target, false).await;
    let report = apply_pair(
        applying.input(&ops, IdPolicy::Keep),
        &CancelFlag::default(),
        &mut |_| {},
    )
    .await
    .unwrap();
    assert_eq!(report.applied, 1);
    assert_eq!(applying.target.recorded().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_item_names_its_destination_side_per_direction() {
    let rig = Rig::plain(vec![doc! {"_id": 1}], vec![doc! {"_id": 2}], true).await;
    let out = plan(&rig, &[op(k(1), Insert, ST), op(k(2), Insert, TS)]).await;
    assert_eq!(out.items[0].destination(), Some(Side::Target));
    assert_eq!(out.items[1].destination(), Some(Side::Source));
    let stale = ExportItem::Failed {
        op: op(k(1), Insert, ST),
        error: crate::error::CoreError::NotFound,
    };
    assert_eq!(stale.destination(), None);
}

#[test]
fn export_skip_maps_the_apply_reasons() {
    assert_eq!(ExportSkip::from(SkipReason::Stale), ExportSkip::Stale);
    assert_eq!(
        ExportSkip::from(SkipReason::NotPlanned),
        ExportSkip::NotPlanned
    );
}
