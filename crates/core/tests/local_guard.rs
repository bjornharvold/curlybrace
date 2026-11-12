//! Tier 1 (local server): the index-miss guard against a real MongoDB. Writes only to throwaway
//! `cbit_*` databases. Run with `scripts/integration-tests.sh`.
//!
//! Verdicts asserted here were observed on mongod 9.0.4.

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::guard::{GUARD_BUDGET_MS, GUARD_MIN_DOCS, GuardReport, GuardTarget, check};
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::read_only_store::ReadOnlyStore;
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use mongodb::{Client, IndexModel};
use support::{insert_labelled, local_enabled, local_mongo};

const BIG: i32 = 2_000;

fn target(ns: &Namespace, method: &str, query: Query) -> GuardTarget {
    GuardTarget {
        ns: ns.clone(),
        method: method.to_string(),
        query,
    }
}

fn filter_on(field: &str, value: i32) -> Query {
    Query {
        filter: doc! { field: value },
        ..Query::default()
    }
}

/// `n` documents with `a` cycling through 0..10, and `b` and `c` unique.
fn docs(n: i32) -> Vec<Document> {
    (0..n)
        .map(|i| doc! { "a": i % 10, "b": i, "c": (i * 7919) % n, "pad": "x" })
        .collect()
}

async fn create_index(client: &Client, ns: &Namespace, keys: Document) {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .create_index(IndexModel::builder().keys(keys).build())
        .await
        .unwrap();
}

async fn run(store: &dyn DocumentStore, t: GuardTarget, min_docs: Option<u64>) -> GuardReport {
    check(
        store,
        vec![t],
        min_docs,
        Duration::from_millis(GUARD_BUDGET_MS),
    )
    .await
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn unindexed_filter_is_a_finding_until_an_index_exists() {
    if !local_enabled("unindexed_filter_is_a_finding_until_an_index_exists") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(BIG)).await;
    let t = target(&ns, "deleteMany", filter_on("c", 5));

    let report = run(&store, t.clone(), Some(GUARD_MIN_DOCS)).await;
    assert_eq!(report.unchecked, 0);
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert_eq!(report.findings[0].kind, VerdictKind::CollectionScan);
    assert_eq!(report.findings[0].method, t.method);
    assert_eq!(report.findings[0].ns, ns);

    create_index(&client, &ns, doc! { "c": 1 }).await;
    let report = run(&store, t, Some(GUARD_MIN_DOCS)).await;
    assert_eq!(
        report,
        GuardReport::default(),
        "an index silences the guard"
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn explain_plan_does_not_run_the_query() {
    if !local_enabled("explain_plan_does_not_run_the_query") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(BIG)).await;
    let explain = store
        .explain_plan(&ns, &filter_on("c", 5))
        .await
        .expect("explain_plan");
    assert!(explain.plan_only);
    assert!(
        explain.document.get("executionStats").is_none(),
        "plan-only explain must not carry executionStats: {:?}",
        explain.document
    );
    assert!(explain.document.get_document("queryPlanner").is_ok());
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn small_collection_is_below_the_threshold() {
    if !local_enabled("small_collection_is_below_the_threshold") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("small");
    let just_under = i32::try_from(GUARD_MIN_DOCS).unwrap() - 1;
    insert_labelled(&client, &ns, &docs(just_under)).await;
    let t = target(&ns, "find", filter_on("c", 5));

    let report = run(&store, t.clone(), Some(GUARD_MIN_DOCS)).await;
    assert_eq!(
        report,
        GuardReport::default(),
        "999 documents are below the threshold"
    );

    let report = run(&store, t, None).await;
    assert_eq!(
        report.findings.len(),
        1,
        "no threshold reports the scan: {report:?}"
    );
    assert_eq!(report.findings[0].kind, VerdictKind::CollectionScan);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn text_query_without_a_text_index_is_unchecked() {
    if !local_enabled("text_query_without_a_text_index_is_unchecked") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(BIG)).await;
    let q = Query {
        filter: doc! { "$text": { "$search": "x" } },
        ..Query::default()
    };
    let report = run(&store, target(&ns, "find", q), Some(GUARD_MIN_DOCS)).await;
    assert!(report.findings.is_empty(), "{report:?}");
    assert_eq!(report.unchecked, 1);
}

/// With an index on `a` and a sort on the unindexed `c`, the winning plan is an `IXSCAN` on `a`
/// feeding a blocking `SORT`, which `classify` reports as `SortNotCovered` (the same verdict
/// `local_explain` pins for the executed explain).
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn sort_on_an_unindexed_field_is_sort_not_covered() {
    if !local_enabled("sort_on_an_unindexed_field_is_sort_not_covered") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(BIG)).await;
    create_index(&client, &ns, doc! { "a": 1 }).await;
    let q = Query {
        sort: Some(doc! { "c": -1 }),
        ..filter_on("a", 3)
    };
    let report = run(&store, target(&ns, "find", q), Some(GUARD_MIN_DOCS)).await;
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert_eq!(report.findings[0].kind, VerdictKind::SortNotCovered);
}

/// A read-only connection (the store wrapped in `ReadOnlyStore`, locked) can run the check: the
/// guard only explains and counts. The server here has no auth, so a role-limited user would not
/// be enforced; the wrapper is the read-only mechanism the app itself uses.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn read_only_connection_can_run_the_check() {
    if !local_enabled("read_only_connection_can_run_the_check") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &docs(BIG)).await;
    let locked = Arc::new(AtomicBool::new(true));
    let read_only = ReadOnlyStore::wrap(Arc::new(store as MongoStore), locked);
    assert!(read_only.is_read_only());
    let t = target(&ns, "find", filter_on("c", 5));
    let report = run(read_only.as_ref(), t, Some(GUARD_MIN_DOCS)).await;
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert_eq!(report.findings[0].kind, VerdictKind::CollectionScan);
}
