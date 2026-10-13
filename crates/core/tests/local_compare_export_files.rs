//! Tier 1 (local server): JSON plans, CSV reports, cancelling an export and the one-destination
//! rule, against a real MongoDB. No `mongosh` needed. Writes only to throwaway `cbit_*`
//! databases; needs `CURLYBRACE_IT=1` and an explicit `CURLYBRACE_IT_MONGODB_URI` (see
//! docs/superpowers/notes/testing.md), otherwise each test skips with a message.

mod support;

use std::collections::BTreeSet;

use bson::{Bson, doc};
use curlybrace_core::compare::apply::{ApplyKind, ApplyOp, Direction};
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, ExportFormat, ReportRequest, ReportTarget,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use serde_json::{Value, json};
use support::compare_flow::{SESSION, compare, key, keys, open, pair, send};
use support::export_flow::{
    all_ops, enabled, export, export_ok, parse_csv, read, report, request, seed,
};
use support::{local_runtime, next_compare};

const SECRET_SRC: &str = "SENTINEL-VALUE-FROM-SOURCE-4711";
const SECRET_DST: &str = "SENTINEL-VALUE-FROM-TARGET-0815";

fn id_of(v: &Value) -> Value {
    v.get("id")
        .cloned()
        .or_else(|| v.pointer("/document/_id").cloned())
        .expect("an operation with an id")
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn plan_json_parses_and_matches_the_planned_ops() {
    if !enabled("plan_json_parses_and_matches_the_planned_ops") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("j_src"), dbs.ns("j_dst"));
    seed(
        &client,
        &src,
        (1..=4).map(|i| doc! { "_id": i, "v": i }).collect(),
    )
    .await;
    seed(
        &client,
        &dst,
        vec![doc! { "_id": 2, "v": 20 }, doc! { "_id": 5, "v": 5 }],
    )
    .await;
    open(&mut rt, &config).await;
    compare(
        &mut rt,
        1,
        vec![pair(1, src.clone(), dst.clone(), &["_id"])],
    )
    .await;
    let ops = all_ops(&mut rt, 1).await;
    assert_eq!(ops.len(), 5);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.json");
    let mut req = request(ExportFormat::JsonPlan, ops.clone());
    req.label = "label with \"quotes\" and \u{2028}".into();
    let outcome = export_ok(&mut rt, 2, req, &path).await;
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (5, 0, 0)
    );
    assert_eq!(
        outcome.bytes,
        std::fs::metadata(&path).unwrap().len(),
        "the reported size is the file size"
    );

    let plan: Value = serde_json::from_str(&read(&path)).expect("the plan is valid JSON");
    let header = &plan["header"];
    assert_eq!(header["format"], "curlybrace-compare-plan");
    assert_eq!(header["version"], 1);
    assert_eq!(header["complete"], true);
    assert_eq!(header["label"], "label with \"quotes\" and \u{2028}");
    let counts = &header["counts"];
    assert_eq!(
        (
            counts["insert"].as_u64(),
            counts["replace"].as_u64(),
            counts["delete"].as_u64(),
            counts["updateFields"].as_u64(),
            counts["skipped"].as_u64()
        ),
        (Some(3), Some(1), Some(1), Some(0), Some(0))
    );
    let pairs = plan["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs[0]["source"],
        json!({ "db": src.db, "collection": src.collection })
    );
    assert_eq!(
        pairs[0]["target"],
        json!({ "db": dst.db, "collection": dst.collection })
    );
    assert_eq!(pairs[0]["key"], json!(["_id"]));
    let got: BTreeSet<(String, String)> = pairs[0]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["op"].as_str().unwrap().to_string(), id_of(o).to_string()))
        .collect();
    let id = |n: i32| json!({ "$numberInt": n.to_string() }).to_string();
    let want: BTreeSet<(String, String)> = [
        ("insert", 1),
        ("replace", 2),
        ("insert", 3),
        ("insert", 4),
        ("delete", 5),
    ]
    .into_iter()
    .map(|(k, n)| (k.to_string(), id(n)))
    .collect();
    assert_eq!(got, want, "{plan}");
    assert!(!read(&path).contains("mongodb://") && !read(&path).contains("127.0.0.1"));
    // The export wrote nothing to the databases.
    assert_eq!(
        support::compare_flow::all_docs(&client, &dst).await,
        vec![doc! { "_id": 2, "v": 20 }, doc! { "_id": 5, "v": 5 }]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn the_exported_script_has_one_destination_side() {
    if !enabled("the_exported_script_has_one_destination_side") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (a_src, a_dst, b_src, b_dst) = (
        dbs.ns("a_src"),
        dbs.ns("a_dst"),
        dbs.ns("b_src"),
        dbs.ns("b_dst"),
    );
    seed(&client, &a_src, vec![doc! { "_id": 1 }]).await;
    seed(&client, &a_dst, vec![]).await;
    seed(&client, &b_src, vec![]).await;
    seed(&client, &b_dst, vec![doc! { "_id": 2 }]).await;
    open(&mut rt, &config).await;
    compare(
        &mut rt,
        1,
        vec![
            pair(1, a_src, a_dst.clone(), &["_id"]),
            pair(2, b_src.clone(), b_dst, &["_id"]),
        ],
    )
    .await;
    let forward = support::compare_flow::op(1, key(1), ApplyKind::Insert);
    let backward = ApplyOp {
        direction: Direction::TargetToSource,
        ..support::compare_flow::op(2, key(2), ApplyKind::Insert)
    };
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("mixed.js");
    let refused = export(
        &mut rt,
        2,
        request(
            ExportFormat::Mongosh,
            vec![forward.clone(), backward.clone()],
        ),
        &script,
    )
    .await;
    match refused {
        Err((context, CoreError::Unsupported(why))) => {
            assert_eq!(context, "compare_export");
            assert!(why.contains("one direction"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!script.exists(), "a refused export leaves no file");

    // The same request is fine as a JSON plan, which says where each operation goes.
    let plan = dir.path().join("mixed.json");
    let outcome = export_ok(
        &mut rt,
        3,
        request(ExportFormat::JsonPlan, vec![forward.clone(), backward]),
        &plan,
    )
    .await;
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (2, 0, 0)
    );
    let parsed: Value = serde_json::from_str(&read(&plan)).unwrap();
    let destinations: Vec<&str> = parsed["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|p| p["operations"].as_array().unwrap())
        .map(|o| o["destination"].as_str().unwrap())
        .collect();
    assert_eq!(destinations, vec!["target", "source"]);

    // One direction as a script works and says which side it writes.
    let one = dir.path().join("one.js");
    export_ok(
        &mut rt,
        4,
        request(ExportFormat::Mongosh, vec![forward]),
        &one,
    )
    .await;
    assert!(read(&one).contains("destination: target"));
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 2, "no temp files: {leftovers:?}");
}

fn report_request(pair: u32, include_paths: bool, label: &str) -> ReportRequest {
    ReportRequest {
        pairs: vec![pair],
        include_paths,
        label: label.to_string(),
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn report_csv_matches_the_stored_differences() {
    if !enabled("report_csv_matches_the_stored_differences") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("=r_src"), dbs.ns("@r_dst"));
    let user = |k: &str, n: i32| doc! { "_id": bson::oid::ObjectId::new(), "k": k, "n": n };
    seed(
        &client,
        &src,
        vec![
            user("=cmd()", 1),
            user("@x", 1),
            user("same", 1),
            user("a,b\"c\nd", 1),
            user("+1", 1),
        ],
    )
    .await;
    seed(
        &client,
        &dst,
        vec![
            user("@x", 2),
            user("same", 1),
            user("-gone", 1),
            user("\tTab", 1),
        ],
    )
    .await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst, &["k"])]).await;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.csv");
    let label = "=HYPERLINK(\"http://e\",\"c\"),x\r\n=1+1";
    let (text, outcome) = report(
        &mut rt,
        2,
        report_request(1, false, label),
        ReportTarget::File(path.clone()),
    )
    .await
    .unwrap();
    assert!(text.is_none());
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (6, 0, 0),
        "{outcome:?}"
    );
    let raw = read(&path);
    let rows = parse_csv(&raw);

    // Summary lines are ONE cell each; no cell can start a formula.
    let (comments, table): (Vec<_>, Vec<_>) = rows.iter().partition(|r| r[0].starts_with('#'));
    assert!(comments.iter().all(|r| r.len() == 1), "{comments:?}");
    for cell in rows.iter().flatten() {
        let first = cell.chars().next();
        assert!(
            !matches!(first, Some('=' | '+' | '-' | '@' | '\t' | '\r')),
            "formula-looking cell {cell:?}"
        );
    }
    assert!(
        raw.contains("[\"\"=cmd()\"\"]"),
        "keys arrive quoted: {raw}"
    );
    assert!(
        comments.last().unwrap()[0].starts_with("# end of report: 6"),
        "{comments:?}"
    );
    assert_eq!(
        *table[0],
        vec![
            "pair",
            "source",
            "target",
            "category",
            "key",
            "changed_paths"
        ]
    );
    assert!(table[1..].iter().all(|r| r.len() == 6), "{table:?}");

    // The rows are exactly the stored differences.
    let mut want = BTreeSet::new();
    for (category, name) in [
        (Category::OnlyInSource, "only_in_source"),
        (Category::OnlyInTarget, "only_in_target"),
        (Category::Different, "different"),
    ] {
        for k in keys(&mut rt, 1, category).await {
            let Bson::String(s) = &k.0[0] else {
                panic!("{k:?}")
            };
            want.insert((name.to_string(), s.clone()));
        }
    }
    let got: BTreeSet<(String, String)> = table[1..]
        .iter()
        .map(|r| {
            assert_eq!(r[0], "1");
            let key: Vec<String> = serde_json::from_str(&r[4]).expect("key cell is a JSON array");
            (r[3].clone(), key[0].clone())
        })
        .collect();
    assert_eq!(want.len(), 6);
    assert_eq!(got, want);

    // The clipboard variant is the same text.
    let (clip, clip_outcome) = report(
        &mut rt,
        3,
        report_request(1, false, label),
        ReportTarget::Clipboard,
    )
    .await
    .unwrap();
    assert_eq!(clip.as_deref(), Some(raw.as_str()));
    assert_eq!(clip_outcome.written, 6);
    send(&rt, CompareCommand::Close { session: SESSION });
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn report_with_paths_lists_paths_and_no_values() {
    if !enabled("report_with_paths_lists_paths_and_no_values") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("p_src"), dbs.ns("p_dst"));
    seed(
        &client,
        &src,
        vec![doc! { "_id": 1, "secret": SECRET_SRC, "n": 1, "deep": { "a": [1, 2] }, "same": "s" }],
    )
    .await;
    seed(
        &client,
        &dst,
        vec![doc! { "_id": 1, "secret": SECRET_DST, "n": 2, "deep": { "a": [1, 3] }, "same": "s" }],
    )
    .await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst, &["_id"])]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paths.csv");
    let (_, outcome) = report(
        &mut rt,
        2,
        report_request(1, true, "paths"),
        ReportTarget::File(path.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (1, 0, 0)
    );
    let raw = read(&path);
    for sentinel in [SECRET_SRC, SECRET_DST] {
        assert!(!raw.contains(sentinel), "a value leaked into the report");
    }
    let rows = parse_csv(&raw);
    let row = rows
        .iter()
        .find(|r| r[0] == "1")
        .expect("the different row");
    let paths: Vec<String> = serde_json::from_str(&row[5]).expect("paths cell is a JSON array");
    for want in ["secret", "n", "deep.a[1]"] {
        assert!(paths.iter().any(|p| p == want), "{paths:?}");
    }
    assert!(!paths.iter().any(|p| p == "same"), "{paths:?}");
}

/// Runs `command` and cancels it at its first progress event; returns the ExportDone outcome.
async fn run_and_cancel(
    rt: &mut curlybrace_core::runtime::RuntimeHandle,
    command: CompareCommand,
    run: u64,
    file_exists_at_first_progress: Option<&std::path::Path>,
) -> curlybrace_core::compare::protocol::ExportOutcome {
    send(rt, command);
    let mut cancelled = false;
    loop {
        match next_compare(rt).await {
            CompareEvent::ExportProgress { .. } if !cancelled => {
                if let Some(path) = file_exists_at_first_progress {
                    assert!(path.exists(), "the file exists while the export runs");
                }
                cancelled = true;
                send(
                    rt,
                    CompareCommand::Cancel {
                        session: SESSION,
                        run,
                    },
                );
            }
            CompareEvent::ExportProgress { .. } => {}
            CompareEvent::ExportDone { outcome, .. } => {
                assert!(
                    cancelled,
                    "the export finished before it could be cancelled"
                );
                return outcome;
            }
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn cancel_removes_the_partial_file() {
    if !enabled("cancel_removes_the_partial_file") {
        return;
    }
    const N: i32 = 40_000;
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("c_src"), dbs.ns("c_dst"));
    seed(
        &client,
        &src,
        (0..N).map(|i| doc! { "_id": i, "v": i }).collect(),
    )
    .await;
    seed(&client, &dst, vec![]).await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst.clone(), &["_id"])]).await;
    let ops = all_ops(&mut rt, 1).await;
    assert_eq!(ops.len(), N as usize);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.js");
    let command = CompareCommand::Export {
        session: SESSION,
        run: 2,
        request: request(ExportFormat::Mongosh, ops),
        path: path.clone(),
    };
    let outcome = run_and_cancel(&mut rt, command, 2, Some(&path)).await;
    assert!(
        outcome.cancelled && outcome.partial_removed && !outcome.file_left,
        "{outcome:?}"
    );
    assert!(outcome.not_run > 0 && outcome.bytes == 0, "{outcome:?}");
    assert!(!path.exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "no temp file either"
    );
    assert_eq!(
        support::compare_flow::all_docs(&client, &dst).await.len(),
        0,
        "an export never writes"
    );
    send(&rt, CompareCommand::Close { session: SESSION });
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn cancel_removes_the_partial_report_with_paths() {
    if !enabled("cancel_removes_the_partial_report_with_paths") {
        return;
    }
    const N: i32 = 3_000;
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("cr_src"), dbs.ns("cr_dst"));
    seed(
        &client,
        &src,
        (0..N).map(|i| doc! { "_id": i, "v": 1 }).collect(),
    )
    .await;
    seed(
        &client,
        &dst,
        (0..N).map(|i| doc! { "_id": i, "v": 2 }).collect(),
    )
    .await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst, &["_id"])]).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.csv");
    let command = CompareCommand::ExportReport {
        session: SESSION,
        run: 2,
        request: report_request(1, true, "big"),
        target: ReportTarget::File(path.clone()),
    };
    let outcome = run_and_cancel(&mut rt, command, 2, Some(&path)).await;
    assert!(
        outcome.cancelled && outcome.partial_removed && !outcome.file_left,
        "{outcome:?}"
    );
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
