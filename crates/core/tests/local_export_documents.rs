//! Tier 1 (local server): the document export through the real runtime and driver. Writes only
//! to throwaway `cbit_*` databases and a temp dir; needs `CURLYBRACE_IT=1` and an explicit
//! `CURLYBRACE_IT_MONGODB_URI` (see docs/superpowers/notes/testing.md), otherwise each test
//! skips with a message.

mod support;

use std::path::Path;
use std::time::Duration;

use bson::{Bson, Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::export::protocol::{
    DocExportOutcome, DocExportRequest, DocFormat, ExportCommand, ExportEvent, JsonMode,
};
use curlybrace_core::runtime::{Command, Event, RuntimeHandle};
use curlybrace_core::store::Namespace;
use curlybrace_core::test_support::mock_objects::exotic_doc;
use futures::TryStreamExt;
use support::export_flow::{enabled, seed};
use support::{local_runtime, next_event};

async fn connect(rt: &mut RuntimeHandle, config: &ConnectionConfig) {
    rt.tx.send(Command::Connect(config.clone())).unwrap();
    loop {
        match next_event(&mut rt.rx, Duration::from_secs(15), "connect").await {
            Event::Connected { .. } => return,
            Event::ConnectFailed { error, .. } => panic!("connect failed: {error:?}"),
            _ => {}
        }
    }
}

fn request(
    config: &ConnectionConfig,
    ns: &Namespace,
    filter: Document,
    format: DocFormat,
    path: &Path,
) -> DocExportRequest {
    DocExportRequest {
        session: config.id,
        ns: ns.clone(),
        filter,
        format,
        mode: JsonMode::Canonical,
        path: path.to_path_buf(),
    }
}

/// Starts run 1 and returns every export event up to and including the terminal one.
async fn run_export(rt: &mut RuntimeHandle, request: DocExportRequest) -> Vec<ExportEvent> {
    rt.tx
        .send(Command::Export(ExportCommand::Start { run: 1, request }))
        .unwrap();
    collect(rt, false).await
}

async fn collect(rt: &mut RuntimeHandle, cancel_on_progress: bool) -> Vec<ExportEvent> {
    let mut events = Vec::new();
    loop {
        let Event::Export(e) = next_event(&mut rt.rx, Duration::from_secs(60), "an export").await
        else {
            continue;
        };
        if cancel_on_progress && matches!(e, ExportEvent::Progress { .. }) && events.is_empty() {
            rt.tx
                .send(Command::Export(ExportCommand::Cancel { run: 1 }))
                .unwrap();
        }
        let terminal = matches!(e, ExportEvent::Done { .. } | ExportEvent::Failed { .. });
        events.push(e);
        if terminal {
            return events;
        }
    }
}

fn done(events: &[ExportEvent]) -> DocExportOutcome {
    match events.last() {
        Some(ExportEvent::Done { outcome, .. }) => *outcome,
        other => panic!("expected Done, got {other:?}"),
    }
}

async fn find_all(client: &mongodb::Client, ns: &Namespace, filter: Document) -> Vec<Document> {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find(filter)
        .sort(doc! { "_id": 1 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

/// `exotic_doc()` with a given `_id` and without the NaN (NaN is never equal to itself).
fn exotic_with_id(id: i32) -> Document {
    let mut d = exotic_doc();
    d.insert("_id", id);
    d.remove("nan");
    d
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn canonical_ndjson_round_trips_every_bson_type() {
    if !enabled("canonical_ndjson_round_trips_every_bson_type") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let ns = dbs.ns("exotic");
    let mut with_nan = exotic_doc();
    with_nan.insert("_id", 0_i32);
    let mut docs = vec![with_nan];
    docs.extend((1..=20).map(exotic_with_id));
    // Inserted out of order: the file must still come out in ascending `_id` order.
    let mut shuffled = docs.clone();
    shuffled.reverse();
    seed(&client, &ns, shuffled).await;
    connect(&mut rt, &config).await;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.ndjson");
    let events = run_export(
        &mut rt,
        request(&config, &ns, doc! {}, DocFormat::Ndjson, &path),
    )
    .await;
    let outcome = done(&events);
    assert_eq!((outcome.written, outcome.cancelled), (21, false));
    assert_eq!(outcome.bytes, std::fs::metadata(&path).unwrap().len());

    let text = std::fs::read_to_string(&path).unwrap();
    let parsed: Vec<Document> = text
        .lines()
        .map(|l| editor_json_to_doc(l).expect("every line parses"))
        .collect();
    let found = find_all(&client, &ns, doc! {}).await;
    assert_eq!(parsed.len(), found.len());
    // NaN: not equal to itself, so check it apart and compare the rest.
    let nan_of = |d: &Document| d.get_f64("nan").map(f64::is_nan);
    assert_eq!(nan_of(&parsed[0]), Ok(true));
    assert_eq!(nan_of(&found[0]), Ok(true));
    let strip = |mut d: Document| {
        d.remove("nan");
        d
    };
    let parsed: Vec<Document> = parsed.into_iter().map(strip).collect();
    let found: Vec<Document> = found.into_iter().map(strip).collect();
    assert_eq!(parsed, found, "canonical keeps every BSON type");
    assert!(matches!(parsed[1].get("int64"), Some(Bson::Int64(_))));
    assert!(matches!(parsed[1].get("int32"), Some(Bson::Int32(_))));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn json_array_with_a_filter_writes_only_the_matching_documents() {
    if !enabled("json_array_with_a_filter_writes_only_the_matching_documents") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let ns = dbs.ns("filtered");
    seed(
        &client,
        &ns,
        (1..=30)
            .map(|i| doc! { "_id": i, "even": i % 2 == 0, "n": i })
            .collect(),
    )
    .await;
    connect(&mut rt, &config).await;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.json");
    let filter = doc! { "even": true };
    let events = run_export(
        &mut rt,
        request(&config, &ns, filter.clone(), DocFormat::JsonArray, &path),
    )
    .await;
    assert_eq!(done(&events).written, 15);

    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).expect("valid JSON");
    let parsed: Vec<Document> = value
        .as_array()
        .expect("a JSON array")
        .iter()
        .map(|v| editor_json_to_doc(&v.to_string()).unwrap())
        .collect();
    assert_eq!(parsed, find_all(&client, &ns, filter).await);
    assert!(parsed.iter().all(|d| d.get_bool("even") == Ok(true)));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn cancelling_mid_export_leaves_no_file() {
    if !enabled("cancelling_mid_export_leaves_no_file") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let ns = dbs.ns("big");
    seed(
        &client,
        &ns,
        (0..200_000)
            .map(|i| doc! { "_id": i, "pad": "x".repeat(64) })
            .collect(),
    )
    .await;
    connect(&mut rt, &config).await;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.ndjson");
    rt.tx
        .send(Command::Export(ExportCommand::Start {
            run: 1,
            request: request(&config, &ns, doc! {}, DocFormat::Ndjson, &path),
        }))
        .unwrap();
    // Cancel as soon as the first progress event arrives.
    let events = collect(&mut rt, true).await;
    let outcome = done(&events);
    assert!(
        outcome.cancelled,
        "the run finished before the cancel took effect: {outcome:?}"
    );
    assert!(!outcome.file_left);
    assert!(!path.exists(), "a cancelled export leaves no file");
    assert!(outcome.written < 200_000);
}
