//! Export through the runtime: what the file holds for each format, the refusals, and the shared
//! rig. In-memory stores; temp directories are created and removed by each test.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bson::{Bson, doc};
use tempfile::TempDir;

use super::apply_tests::{ApplyStore, op, source_store, target_store, within};
use super::tests::{MapConnector, Tx, collect_run, next_compare, ns, pair, same_pair, start_with};
use crate::compare::apply::{ApplyKind, ApplyOp, Direction, IdPolicy};
use crate::compare::apply_engine::ApplyInput;
use crate::compare::engine::{CancelFlag, PairInput, run_pair};
use crate::compare::export::mongosh::{ScriptWriter, ShellOp};
use crate::compare::export::plan::{ExportItem, plan_pair_export};
use crate::compare::export::{ExportCounts, ExportHeader, OpMeta};
use crate::compare::key::KeySpec;
use crate::compare::protocol::{
    BulkApply, CompareCommand, CompareEvent, ExportFormat, ExportOutcome, ExportRequest, PairId,
    PairOutcome, PairSpec,
};
use crate::compare::results::{Budget, Category, Limits};
use crate::connection::ConnectionConfig;
use crate::error::CoreError;
use crate::runtime::{Command, RuntimeConfig, RuntimeHandle};
use crate::store::{CollectionKind, DocumentStore, Namespace};
use crate::test_support::InMemoryStore;

pub(super) const SRC_NAME: &str = "SRC-CONNECTION-NAME-7c1";
pub(super) const TGT_NAME: &str = "TGT-CONNECTION-NAME-7c2";
pub(super) const SRC_URI: &str = "mongodb://srcuser@src-host-7c1.internal:27017/hidden-src";
pub(super) const TGT_URI: &str = "mongodb://tgtuser@tgt-host-7c2.internal:27017/hidden-tgt";
pub(super) const LABEL: &str = "LABEL-1";

pub(super) struct ExRig {
    pub tx: Tx,
    pub h: RuntimeHandle,
    pub source: Arc<ApplyStore>,
    pub target: Arc<ApplyStore>,
    pub dir: TempDir,
}

impl ExRig {
    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

pub(super) fn c2_pair() -> PairSpec {
    pair(2, ns("d", "c2"), ns("d", "c2"))
}

pub(super) async fn rig() -> ExRig {
    rig_with(
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
        vec![same_pair(1), c2_pair()],
    )
    .await
}

/// A session (id 1) with distinctive connection names, whose `pairs` have been compared.
pub(super) async fn rig_with(
    source: Arc<ApplyStore>,
    target: Arc<ApplyStore>,
    pairs: Vec<PairSpec>,
) -> ExRig {
    let connector = MapConnector {
        stores: std::collections::HashMap::from([
            (
                SRC_URI.to_string(),
                source.clone() as Arc<dyn DocumentStore>,
            ),
            (
                TGT_URI.to_string(),
                target.clone() as Arc<dyn DocumentStore>,
            ),
        ]),
        ..MapConnector::default()
    };
    let (tx, mut h) = start_with(connector, RuntimeConfig::default());
    tx.send(Command::Compare(CompareCommand::Open {
        session: 1,
        source: ConnectionConfig::new(SRC_NAME, SRC_URI),
        target: ConnectionConfig::new(TGT_NAME, TGT_URI),
    }))
    .unwrap();
    assert_eq!(
        next_compare(&mut h).await,
        CompareEvent::Opened { session: 1 }
    );
    super::tests::send_start(&tx, 1, pairs);
    let done = collect_run(&mut h).await;
    assert!(
        done.iter()
            .all(|(_, o)| matches!(o, PairOutcome::Completed(_)))
    );
    ExRig {
        tx,
        h,
        source,
        target,
        dir: tempfile::tempdir().unwrap(),
    }
}

pub(super) fn request(format: ExportFormat, ops: Vec<ApplyOp>) -> ExportRequest {
    ExportRequest {
        format,
        label: LABEL.into(),
        id_policy: IdPolicy::Keep,
        ops,
        bulk: Vec::new(),
    }
}

pub(super) fn send_export(tx: &Tx, run: u64, request: ExportRequest, path: &Path) {
    tx.send(Command::Compare(CompareCommand::Export {
        session: 1,
        run,
        request,
        path: path.to_path_buf().into(),
    }))
    .unwrap();
}

/// Events up to and including the first `ExportDone` or `Failed`.
pub(super) async fn collect_export(h: &mut RuntimeHandle) -> Vec<CompareEvent> {
    let mut events = Vec::new();
    loop {
        let event = next_compare(h).await;
        let last = matches!(
            event,
            CompareEvent::ExportDone { .. } | CompareEvent::Failed { .. }
        );
        events.push(event);
        if last {
            return events;
        }
    }
}

pub(super) fn outcome_of(events: &[CompareEvent]) -> ExportOutcome {
    match events.last() {
        Some(CompareEvent::ExportDone { outcome, .. }) => *outcome,
        other => panic!("{other:?}"),
    }
}

/// Runs one export to its end and returns its events.
pub(super) async fn export(
    r: &mut ExRig,
    run: u64,
    request: ExportRequest,
    path: &Path,
) -> Vec<CompareEvent> {
    send_export(&r.tx, run, request, path);
    within(collect_export(&mut r.h)).await
}

pub(super) fn standard_ops() -> Vec<ApplyOp> {
    vec![
        op(1, 1, ApplyKind::Insert),
        op(1, 2, ApplyKind::Replace),
        op(2, 1, ApplyKind::Insert),
    ]
}

/// The items the planner yields for `ops` of `spec` over fresh copies of the rig's stores.
pub(super) async fn planned(
    spec: &PairSpec,
    source: InMemoryStore,
    target: InMemoryStore,
    ops: &[ApplyOp],
) -> Vec<ExportItem> {
    let (source, target): (Arc<dyn DocumentStore>, Arc<dyn DocumentStore>) =
        (Arc::new(source), Arc::new(target));
    let input = PairInput {
        source: source.clone(),
        source_ns: spec.source.clone(),
        source_filter: doc! {},
        target: target.clone(),
        target_ns: spec.target.clone(),
        target_filter: doc! {},
        key: KeySpec::id(),
        rules: spec.rules.clone(),
        budget: Budget::new(Limits::default()),
    };
    let (_, _, results) = run_pair(input, &CancelFlag::default(), &mut |_| {})
        .await
        .unwrap();
    let apply = ApplyInput {
        spec,
        results: &results,
        source,
        target,
        ops,
        policy: IdPolicy::Keep,
    };
    let mut items = Vec::new();
    let mut sink = |batch: Vec<ExportItem>| {
        items.extend(batch);
        Ok(())
    };
    plan_pair_export(apply, &CancelFlag::default(), &mut sink, &mut |_| {})
        .await
        .unwrap();
    items
}

/// The script the Task 1 writer renders for sections of `(db, collection, items)`.
fn oracle_script(counts: ExportCounts, sections: &[(&str, &str, Vec<ExportItem>)]) -> Vec<u8> {
    let header = ExportHeader {
        label: LABEL.into(),
        counts,
        id_policy: IdPolicy::Keep,
        count_width: 1,
        complete: true,
        destination: Some(crate::compare::protocol::Side::Target),
    };
    let mut w = ScriptWriter::new(Vec::new());
    w.begin(&header).unwrap();
    for (db, collection, items) in sections {
        w.collection(db, collection).unwrap();
        for item in items {
            match item {
                ExportItem::Write {
                    op,
                    write,
                    source_fp,
                    target_fp,
                } => {
                    let meta = meta(op, source_fp, target_fp);
                    w.op(&meta, &ShellOp::from(write.clone())).unwrap();
                }
                ExportItem::Skipped {
                    op,
                    reason,
                    source_fp,
                    target_fp,
                } => w
                    .skipped(&meta(op, source_fp, target_fp), op.kind, *reason)
                    .unwrap(),
                ExportItem::Failed { .. } => {}
            }
        }
        w.end_collection().unwrap();
    }
    w.finish().unwrap()
}

fn meta(
    op: &ApplyOp,
    source: &Option<crate::compare::fingerprint::Fingerprint>,
    target: &Option<crate::compare::fingerprint::Fingerprint>,
) -> OpMeta {
    OpMeta {
        direction: op.direction,
        key: op.key.clone(),
        source: *source,
        target: *target,
    }
}

pub(super) fn text(path: &Path) -> String {
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

#[tokio::test]
async fn a_script_export_equals_the_writer_output_for_the_planned_items() {
    let mut r = rig().await;
    let path = r.path("plan.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    )
    .await;
    let shape: Vec<&str> = events
        .iter()
        .map(|e| match e {
            CompareEvent::ExportProgress { run: 5, .. } => "progress",
            CompareEvent::ExportDone { run: 5, .. } => "done",
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(shape, ["progress", "progress", "done"]);
    let first = planned(
        &same_pair(1),
        source_store(),
        target_store(),
        &[op(1, 1, ApplyKind::Insert), op(1, 2, ApplyKind::Replace)],
    )
    .await;
    let second = planned(
        &c2_pair(),
        source_store(),
        target_store(),
        &[op(2, 1, ApplyKind::Insert)],
    )
    .await;
    let counts = ExportCounts {
        insert: 2,
        replace: 1,
        ..ExportCounts::default()
    };
    let expected = oracle_script(counts, &[("d", "c", first), ("d", "c2", second)]);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&bytes),
        String::from_utf8_lossy(&expected)
    );
    let outcome = outcome_of(&events);
    assert_eq!(
        outcome,
        ExportOutcome {
            written: 3,
            bytes: bytes.len() as u64,
            ..ExportOutcome::default()
        }
    );
    let totals: Vec<(u64, u64)> = events
        .iter()
        .filter_map(|e| match e {
            CompareEvent::ExportProgress { done, total, .. } => Some((*done, *total)),
            _ => None,
        })
        .collect();
    assert_eq!(totals, [(2, 3), (3, 3)]);
    super::apply_tests::assert_silent(&mut r.h).await;
}

#[tokio::test]
async fn a_json_plan_export_lists_the_planned_operations_per_pair() {
    let mut r = rig().await;
    let path = r.path("plan.json");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::JsonPlan, standard_ops()),
        &path,
    )
    .await;
    let outcome = outcome_of(&events);
    assert_eq!(outcome.written, 3);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(outcome.bytes, bytes.len() as u64);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["header"]["label"], LABEL);
    assert_eq!(json["header"]["counts"]["insert"], 2);
    assert_eq!(json["header"]["counts"]["replace"], 1);
    assert_eq!(json["header"]["counts"]["skipped"], 0);
    let pairs = json["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(
        pairs[0]["source"],
        serde_json::json!({"db": "d", "collection": "c"})
    );
    let kinds = |p: &serde_json::Value| -> Vec<String> {
        p["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["op"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(kinds(&pairs[0]), ["insert", "replace"]);
    assert_eq!(kinds(&pairs[1]), ["insert"]);
}

#[tokio::test]
async fn refusals_leave_no_file_and_free_the_slot() {
    let mut r = rig().await;
    let dir = r.path("a-directory");
    std::fs::create_dir(&dir).unwrap();
    let cases: Vec<(PathBuf, ExportRequest, &str)> = vec![
        (
            PathBuf::from("relative.js"),
            request(ExportFormat::Mongosh, standard_ops()),
            "relative",
        ),
        (
            r.path("missing-parent/x.js"),
            request(ExportFormat::Mongosh, standard_ops()),
            "parent",
        ),
        (
            dir.clone(),
            request(ExportFormat::Mongosh, standard_ops()),
            "dir",
        ),
        (
            r.path("empty.js"),
            request(ExportFormat::Mongosh, Vec::new()),
            "empty",
        ),
        (
            r.path("unknown.js"),
            request(ExportFormat::Mongosh, vec![op(9, 1, ApplyKind::Insert)]),
            "unknown pair",
        ),
        (
            r.path("dup.js"),
            request(
                ExportFormat::Mongosh,
                vec![op(1, 1, ApplyKind::Insert), op(1, 1, ApplyKind::Replace)],
            ),
            "duplicate",
        ),
    ];
    for (path, request, why) in cases {
        send_export(&r.tx, 5, request, &path);
        match next_compare(&mut r.h).await {
            CompareEvent::Failed {
                context: "compare_export",
                ..
            } => {}
            other => panic!("{why}: {other:?}"),
        }
        assert!(!path.exists() || path == dir, "{why}: no file");
    }
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
    let ok = r.path("ok.js");
    let events = export(
        &mut r,
        6,
        request(ExportFormat::Mongosh, standard_ops()),
        &ok,
    )
    .await;
    assert_eq!(outcome_of(&events).written, 3);
}

#[tokio::test]
async fn an_invalid_path_is_unsupported_and_refused_before_the_slot_is_taken() {
    let mut r = rig().await;
    send_export(
        &r.tx,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        Path::new("relative.js"),
    );
    match next_compare(&mut r.h).await {
        CompareEvent::Failed { error, .. } => {
            assert_eq!(error, CoreError::Unsupported("invalid export path".into()));
        }
        other => panic!("{other:?}"),
    }
    // A Start right behind it is accepted: the refused export never held the slot.
    super::tests::send_start(&r.tx, 6, vec![same_pair(1)]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
}

#[tokio::test]
async fn a_view_destination_is_refused_after_the_slot_and_leaves_no_file() {
    let target = ApplyStore::with_kinds(target_store(), vec![("c", CollectionKind::View)]);
    let mut r = rig_with(
        ApplyStore::new(source_store()),
        target,
        vec![same_pair(1), c2_pair()],
    )
    .await;
    let path = r.path("view.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    )
    .await;
    assert!(matches!(
        events.as_slice(),
        [CompareEvent::Failed {
            context: "compare_export",
            error: CoreError::ApplyTargetNotWritable,
            ..
        }]
    ));
    assert!(!path.exists());
    let ok = r.path("ok.js");
    let events = export(
        &mut r,
        6,
        request(ExportFormat::Mongosh, vec![op(2, 1, ApplyKind::Insert)]),
        &ok,
    )
    .await;
    assert_eq!(outcome_of(&events).written, 1, "the slot was freed");
}

#[tokio::test]
async fn the_file_holds_no_connection_name_uri_or_host() {
    let mut r = rig().await;
    for (format, name) in [
        (ExportFormat::Mongosh, "a.js"),
        (ExportFormat::JsonPlan, "a.json"),
    ] {
        let path = r.path(name);
        export(&mut r, 5, request(format, standard_ops()), &path).await;
        let content = text(&path);
        for secret in [
            SRC_NAME,
            TGT_NAME,
            "mongodb://",
            "src-host",
            "tgt-host",
            "srcuser",
            "tgtuser",
            "hidden-src",
            "hidden-tgt",
        ] {
            assert!(!content.contains(secret), "{name} leaks {secret}");
        }
    }
}

#[tokio::test]
async fn a_hostile_label_and_collection_name_cannot_add_statements_or_lines() {
    let hostile_coll = "c\"\n);\ndb.dropDatabase();//\u{2028}x";
    let hostile_ns = Namespace::new("d", hostile_coll);
    let source = InMemoryStore::new().with_docs(&hostile_ns, vec![doc! {"_id": 1}]);
    let mut r = rig_with(
        ApplyStore::new(source),
        ApplyStore::new(InMemoryStore::new()),
        vec![pair(1, hostile_ns.clone(), hostile_ns)],
    )
    .await;
    let path = r.path("hostile.js");
    let mut req = request(ExportFormat::Mongosh, vec![op(1, 1, ApplyKind::Insert)]);
    req.label = "lbl\r\n//x\ndb.dropDatabase();\u{2029}INJECTED".into();
    export(&mut r, 5, req, &path).await;
    let content = text(&path);
    assert_eq!(content.matches(".bulkWrite(").count(), 1);
    assert!(
        !content.lines().any(|l| l.starts_with("db.dropDatabase")),
        "no injected statement"
    );
    for line in content.lines().filter(|l| l.contains("INJECTED")) {
        assert!(
            line.starts_with("//"),
            "the label stays on a comment line: {line}"
        );
    }
    let statements = content.lines().filter(|l| l.starts_with("db.")).count();
    assert_eq!(statements, 1, "one statement for the one collection");
}

#[tokio::test]
async fn several_pairs_in_one_direction_give_one_statement_per_destination() {
    let mut r = rig_with(
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
        vec![same_pair(1), pair(3, ns("d", "c2"), ns("d", "c"))],
    )
    .await;
    let ops = vec![op(1, 1, ApplyKind::Insert), op(3, 1, ApplyKind::Insert)];
    let path = r.path("multi.js");
    export(&mut r, 5, request(ExportFormat::Mongosh, ops), &path).await;
    let content = text(&path);
    assert_eq!(content.matches(".bulkWrite(").count(), 1);
    assert_eq!(content.matches("insertOne").count(), 2);
    assert_eq!(
        content
            .matches("// destination: target side of the comparison")
            .count(),
        2,
        "once in the header and once on the statement"
    );
}

#[tokio::test]
async fn stale_and_unsupported_operations_are_listed_as_skipped_never_written() {
    let source = InMemoryStore::new().with_docs(
        &ns("d", "c"),
        vec![
            doc! { "_id": 1, "v": "x" },
            doc! { "_id": 2, "v": "new" },
            doc! { "_id": 3, "u": Bson::Undefined },
        ],
    );
    let mut r = rig_with(
        ApplyStore::new(source),
        ApplyStore::new(target_store()),
        vec![same_pair(1)],
    )
    .await;
    // The target document changed after the comparison: the replace is stale.
    r.target
        .replace(
            &ns("d", "c"),
            &Bson::Int32(2),
            doc! { "_id": 2, "v": "changed" },
        )
        .await
        .unwrap();
    let ops = vec![
        op(1, 1, ApplyKind::Insert),
        op(1, 2, ApplyKind::Replace),
        op(1, 3, ApplyKind::Insert),
    ];
    let path = r.path("skips.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, ops.clone()),
        &path,
    )
    .await;
    let outcome = outcome_of(&events);
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (1, 2, 0)
    );
    let content = text(&path);
    assert!(content.contains("skipped (stale)"));
    assert!(content.contains("skipped (unsupported)"));
    assert_eq!(content.matches("insertOne").count(), 1);
    assert_eq!(content.matches("replaceOne").count(), 0);
    assert!(
        content.contains("skipped 2"),
        "header counts the skipped ops"
    );
    assert_eq!(
        super::export_content_tests::header_total(&content),
        3,
        "the header adds up to the ops requested"
    );
    let plan = r.path("skips.json");
    export(&mut r, 6, request(ExportFormat::JsonPlan, ops), &plan).await;
    let json: serde_json::Value = serde_json::from_str(&text(&plan)).unwrap();
    let kinds: Vec<&str> = json["pairs"][0]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["op"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["insert", "skipped", "skipped"]);
}

#[tokio::test]
async fn field_ops_and_bulk_items_export_like_an_apply_would_plan_them() {
    let mut r = rig().await;
    let mut fields = op(1, 2, ApplyKind::Fields);
    fields.fields = vec![vec![crate::compare::diff::PathSegment::Field("v".into())]];
    let path = r.path("fields.js");
    export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, vec![fields]),
        &path,
    )
    .await;
    let content = text(&path);
    assert!(content.contains("updateOne") && content.contains("$set"));
    let explicit = r.path("explicit.js");
    export(
        &mut r,
        6,
        request(ExportFormat::Mongosh, vec![op(1, 1, ApplyKind::Insert)]),
        &explicit,
    )
    .await;
    let bulk = r.path("bulk.js");
    let mut by_bulk = request(ExportFormat::Mongosh, Vec::new());
    by_bulk.bulk = vec![BulkApply {
        pair: 1 as PairId,
        category: Category::OnlyInSource,
        kind: ApplyKind::Insert,
        direction: Direction::SourceToTarget,
    }];
    export(&mut r, 7, by_bulk, &bulk).await;
    assert_eq!(text(&bulk), text(&explicit));
}

#[tokio::test]
async fn the_stores_are_never_written() {
    let mut r = rig().await;
    r.source
        .panic_bulk
        .store(true, std::sync::atomic::Ordering::SeqCst);
    r.target
        .panic_bulk
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let path = r.path("nowrite.js");
    let mut ops = standard_ops();
    ops.push(op(1, 2, ApplyKind::Delete));
    ops.remove(1);
    let events = export(&mut r, 5, request(ExportFormat::Mongosh, ops), &path).await;
    assert_eq!(outcome_of(&events).failed, 0);
    assert_eq!((r.source.writes(), r.target.writes()), (0, 0));
    assert!(text(&path).starts_with("// complete export"));
}

#[tokio::test]
async fn a_large_export_streams_in_batches_and_gets_its_true_header() {
    let docs: Vec<_> = (0..1200)
        .map(|i| doc! { "_id": i, "v": "x".repeat(120) })
        .collect();
    let big = ns("d", "big");
    let mut r = rig_with(
        ApplyStore::new(InMemoryStore::new().with_docs(&big, docs)),
        ApplyStore::new(InMemoryStore::new()),
        vec![pair(1, big.clone(), big)],
    )
    .await;
    let path = r.path("big.js");
    let mut req = request(ExportFormat::Mongosh, Vec::new());
    req.bulk = vec![BulkApply {
        pair: 1,
        category: Category::OnlyInSource,
        kind: ApplyKind::Insert,
        direction: Direction::SourceToTarget,
    }];
    let events = export(&mut r, 5, req, &path).await;
    let outcome = outcome_of(&events);
    let content = text(&path);
    assert_eq!(outcome.written, 1200);
    assert_eq!(
        outcome.bytes,
        content.len() as u64,
        "the shifted file has its true size"
    );
    assert!(content.contains(
        "// operations: insert 1200, replace    0, delete    0, updateFields    0, skipped    0\n"
    ));
    assert!(content.starts_with("// complete export"));
    assert!(!content.contains("quit(1)"));
    assert_eq!(content.matches("insertOne").count(), 1200);
    assert!(content.ends_with("quit(2); }\n"));
    let dones: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            CompareEvent::ExportProgress { done, .. } => Some(*done),
            _ => None,
        })
        .collect();
    assert!(dones.len() >= 3 && dones.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(dones.last(), Some(&1200));
}
