//! Tier 1 (local server): `explain_find` and `classify` against a real MongoDB. Writes only to
//! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.
//!
//! The classifier's unit tests use fixtures; these tests prove those fixtures resemble what a
//! real server sends. With `CURLYBRACE_CAPTURE=1` each test writes the raw explain document it
//! saw to `src/explain/fixtures/real_<name>.json` so the fixtures can be refreshed.
//! Stage names and shapes asserted here were captured on mongod 9.0.4.
//!
//! Every test takes [`serial`] first: the plan-only test arms a server-global fail point, and a
//! parallel `explain_find` could otherwise consume its single shot.

mod support;

use std::path::PathBuf;

use bson::{Bson, Document, doc};
use curlybrace_core::ejson::doc_to_editor_json;
use curlybrace_core::explain::{Verdict, VerdictKind, classify};
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::store::{DocumentStore, Explain, Namespace, Query};
use mongodb::Client;
use mongodb::IndexModel;
use support::{insert_labelled, local_enabled, local_mongo};

/// Held for the whole body of every test in this file (see the header).
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL.lock().await
}

const CAPTURE_ENV: &str = "CURLYBRACE_CAPTURE";

fn query(filter: Document) -> Query {
    Query {
        filter,
        ..Query::default()
    }
}

/// Replaces `cbit_<12 hex>` database names with `test`.
fn scrub_text(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("cbit_") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 5..];
        let hex = tail
            .chars()
            .take(12)
            .take_while(char::is_ascii_hexdigit)
            .count();
        if hex == 12 {
            out.push_str("test");
            rest = &tail[12..];
        } else {
            out.push_str("cbit_");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

/// Removes what identifies the machine or the throwaway database from a captured document.
fn scrub(value: Bson) -> Bson {
    match value {
        Bson::String(s) => Bson::String(scrub_text(&s)),
        Bson::Array(items) => Bson::Array(items.into_iter().map(scrub).collect()),
        Bson::Document(d) => {
            let is_server_info = d.contains_key("host") && d.contains_key("gitVersion");
            let mut out = Document::new();
            for (k, v) in d {
                let v = match (is_server_info, k.as_str()) {
                    (true, "host") => Bson::String("localhost".into()),
                    (true, "port") => Bson::Int32(27017),
                    (true, "gitVersion") => Bson::String("0".repeat(40)),
                    _ => scrub(v),
                };
                out.insert(k, v);
            }
            Bson::Document(out)
        }
        other => other,
    }
}

/// Writes the explain document to the fixtures directory when capturing is switched on.
fn capture(name: &str, explain: &Explain) {
    if std::env::var(CAPTURE_ENV).as_deref() != Ok("1") {
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/explain/fixtures")
        .join(format!("real_{name}.json"));
    let Bson::Document(scrubbed) = scrub(Bson::Document(explain.document.clone())) else {
        unreachable!("scrub keeps a document a document")
    };
    let mut text = doc_to_editor_json(&scrubbed);
    text.push('\n');
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Explains `q`, captures the document, and classifies it.
async fn verdict_of(
    name: &str,
    store: &MongoStore,
    ns: &Namespace,
    q: &Query,
) -> (Explain, Verdict) {
    let explain = store.explain_find(ns, q).await.expect("explain_find");
    capture(name, &explain);
    let verdict = classify(&explain.document, q).expect("classify");
    (explain, verdict)
}

async fn create_index(client: &Client, ns: &Namespace, keys: Document) {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .create_index(IndexModel::builder().keys(keys).build())
        .await
        .unwrap();
}

/// `n` documents with `a` cycling through 0..10, `b` unique, `c` unique, and some padding.
fn docs(n: i32) -> Vec<Document> {
    (0..n)
        .map(|i| doc! { "a": i % 10, "b": i, "c": (i * 7919) % n, "pad": "x" })
        .collect()
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_collscan_is_collection_scan() {
    if !local_enabled("real_collscan_is_collection_scan") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(300)).await;
    let q = query(doc! { "c": 5 });
    let (explain, v) = verdict_of("collscan", &store, &ns, &q).await;
    assert_eq!(v.kind, VerdictKind::CollectionScan);
    assert!(!explain.plan_only && !v.plan_only);
    assert_eq!(v.docs_examined, 300);
    assert_eq!(v.returned, 1);
    assert_eq!(v.index_name, None);
    assert!(v.suggestion.is_some());
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_indexed_equality_is_index_used() {
    if !local_enabled("real_indexed_equality_is_index_used") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(300)).await;
    create_index(&client, &ns, doc! { "c": 1 }).await;
    let q = query(doc! { "c": 5 });
    let (explain, v) = verdict_of("ixscan", &store, &ns, &q).await;
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!(v.index_name.as_deref(), Some("c_1"));
    assert!(!explain.plan_only && !v.plan_only);
    assert_eq!((v.keys_examined, v.docs_examined, v.returned), (1, 1, 1));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_sort_on_unindexed_field_is_sort_not_covered() {
    if !local_enabled("real_sort_on_unindexed_field_is_sort_not_covered") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(300)).await;
    create_index(&client, &ns, doc! { "a": 1 }).await;
    let q = Query {
        sort: Some(doc! { "c": -1 }),
        ..query(doc! { "a": 3 })
    };
    let (explain, v) = verdict_of("sort", &store, &ns, &q).await;
    assert_eq!(v.kind, VerdictKind::SortNotCovered);
    assert!(!explain.plan_only && !v.plan_only);
    assert_eq!(v.index_name.as_deref(), Some("a_1"));
    assert_eq!(v.returned, 30);
    assert_eq!(v.reasons[0].key, "sort_blocking");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_compound_index_wrong_order_is_inefficient() {
    if !local_enabled("real_compound_index_wrong_order_is_inefficient") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    // 6000 documents with `a` in 0..1000 (6 documents per value) and `b` unique. With
    // {b: 1, a: 1} the equality on `a` cannot narrow the scan: every key in the wide `b` range is
    // read to return the 6 matches.
    let many: Vec<Document> = (0..6000)
        .map(|i| doc! { "a": i % 1000, "b": i, "pad": "x" })
        .collect();
    insert_labelled(&client, &ns, &many).await;
    create_index(&client, &ns, doc! { "b": 1, "a": 1 }).await;
    let q = query(doc! { "a": 3, "b": { "$gte": 0, "$lt": 6000 } });
    let (explain, v) = verdict_of("inefficient", &store, &ns, &q).await;
    assert!(!explain.plan_only && !v.plan_only);
    assert_eq!(v.index_name.as_deref(), Some("b_1_a_1"));
    assert_eq!(v.returned, 6);
    assert!(v.keys_examined >= 100, "keys {}", v.keys_examined);
    assert!(
        v.keys_examined >= 10 * v.returned,
        "keys {}",
        v.keys_examined
    );
    assert_eq!(v.kind, VerdictKind::InefficientIndex, "{v:?}");
    assert_eq!(v.reasons[0].key, "inefficient");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_covered_projection_is_covered() {
    if !local_enabled("real_covered_projection_is_covered") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(300)).await;
    create_index(&client, &ns, doc! { "c": 1 }).await;
    let q = Query {
        projection: Some(doc! { "c": 1, "_id": 0 }),
        ..query(doc! { "c": 5 })
    };
    let (explain, v) = verdict_of("covered", &store, &ns, &q).await;
    assert_eq!(v.kind, VerdictKind::CoveredByIndex);
    assert_eq!(v.index_name.as_deref(), Some("c_1"));
    assert!(!explain.plan_only);
    assert_eq!((v.keys_examined, v.docs_examined, v.returned), (1, 0, 1));
}

/// Pinned (binding): explain on a view is not unavailable. The server folds the view's pipeline
/// into the find, so the call succeeds and the plan is a collection scan over the source.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_view_explain_succeeds_as_collection_scan() {
    if !local_enabled("real_view_explain_succeeds_as_collection_scan") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(300)).await;
    client
        .database(&dbs.db)
        .create_collection("orders_view")
        .view_on("orders")
        .pipeline(vec![doc! { "$match": { "a": { "$gte": 0 } } }])
        .await
        .unwrap();
    let view = dbs.ns("orders_view");
    let q = query(doc! { "c": 5 });
    let explain = match store.explain_find(&view, &q).await {
        Ok(explain) => explain,
        Err(e) => panic!("server behaviour on views changed: explain on a view now fails: {e:?}"),
    };
    capture("view", &explain);
    let v = classify(&explain.document, &q).expect("a view explain classifies");
    assert_eq!(v.kind, VerdictKind::CollectionScan);
    assert!(!explain.plan_only && !v.plan_only);
    assert_eq!((v.docs_examined, v.returned), (300, 1));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_empty_collection_is_empty() {
    if !local_enabled("real_empty_collection_is_empty") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let q = query(doc! { "c": 5 });

    // A collection that does not exist at all.
    let missing = dbs.ns("never_created");
    let (explain, v) = verdict_of("missing", &store, &missing, &q).await;
    assert!(!explain.plan_only);
    assert_eq!((v.docs_examined, v.returned), (0, 0));
    eprintln!("nonexistent collection: {:?}", v.kind);
    assert_eq!(v.kind, VerdictKind::EmptyCollection);

    // A collection that exists but holds no documents.
    let empty = dbs.ns("empty");
    client
        .database(&dbs.db)
        .create_collection("empty")
        .await
        .unwrap();
    let (explain, v) = verdict_of("empty", &store, &empty, &q).await;
    assert!(!explain.plan_only);
    assert_eq!((v.docs_examined, v.returned), (0, 0));
    eprintln!("empty collection: {:?}", v.kind);
    assert_eq!(v.kind, VerdictKind::EmptyCollection);
}

/// Disarms the fail point when dropped, also when an assertion panics (best effort, on a helper
/// thread because Drop can run inside an async runtime).
struct FailPointGuard {
    uri: String,
}

impl Drop for FailPointGuard {
    fn drop(&mut self) {
        let uri = self.uri.clone();
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Runtime::new() else {
                return;
            };
            rt.block_on(async {
                let Ok(client) = Client::with_uri_str(&uri).await else {
                    return;
                };
                let off = doc! { "configureFailPoint": "maxTimeAlwaysTimeOut", "mode": "off" };
                let _ = client.database("admin").run_command(off).await;
                client.shutdown().await;
            });
        })
        .join();
    }
}

/// Forces the time limit with the `maxTimeAlwaysTimeOut` fail point, armed for exactly one
/// command (`times: 1`) so the plan-only retry succeeds. Needs `enableTestCommands=1`; without
/// it the test prints why and returns.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn real_plan_only_fallback() {
    if !local_enabled("real_plan_only_fallback") {
        return;
    }
    let _serial = serial().await;
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(300)).await;
    create_index(&client, &ns, doc! { "c": 1 }).await;
    let admin = client.database("admin");
    let _disarm = FailPointGuard {
        uri: curlybrace_testkit::local_uri(),
    };
    let arm = admin
        .run_command(doc! {
            "configureFailPoint": "maxTimeAlwaysTimeOut",
            "mode": { "times": 1 },
        })
        .await;
    if let Err(e) = arm {
        eprintln!(
            "SKIPPED real_plan_only_fallback: configureFailPoint refused ({e}); start mongod with --setParameter enableTestCommands=1"
        );
        return;
    }
    let q = query(doc! { "c": 5 });
    let outcome = store.explain_find(&ns, &q).await;
    // Disarm before asserting; the guard repeats this if anything panics first.
    let _ = admin
        .run_command(doc! { "configureFailPoint": "maxTimeAlwaysTimeOut", "mode": "off" })
        .await;
    let explain = outcome.expect("the plan-only retry succeeds");
    capture("plan_only", &explain);
    assert!(explain.plan_only);
    assert!(explain.document.get_document("executionStats").is_err());
    let v = classify(&explain.document, &q).unwrap();
    assert!(v.plan_only);
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert_eq!(v.index_name.as_deref(), Some("c_1"));
    assert_eq!((v.keys_examined, v.docs_examined, v.returned), (0, 0, 0));
}
