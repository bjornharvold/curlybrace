//! Tier 1 (local server): aggregate explains against a real MongoDB. Writes only to throwaway
//! `cbit_*` databases. Run with `scripts/integration-tests.sh`, which points at a server you
//! name through `CURLYBRACE_IT_MONGODB_URI`.

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{
    AGGREGATE_EXPLAIN_MAX_TIME_MS, AggHint, AggPlan, AggPlanOutcome, ExplainVerbosity,
    parse_aggregate_explain,
};
use curlybrace_core::pipeline::{Stage, explain_pipeline};
use curlybrace_core::read_only_store::ReadOnlyStore;
use curlybrace_core::store::{DocumentStore, Namespace};
use mongodb::IndexModel;
use support::{insert_labelled, local_enabled, local_mongo};

const MAX_TIME: Duration = Duration::from_millis(AGGREGATE_EXPLAIN_MAX_TIME_MS);

fn stages(texts: &[&str]) -> Vec<Stage> {
    texts
        .iter()
        .map(|t| Stage {
            text: (*t).to_string(),
            enabled: true,
        })
        .collect()
}

/// 400 orders: `k` cycles 0..4 (indexed), `amt` is the index.
fn orders() -> Vec<Document> {
    (0..400)
        .map(|i| doc! { "_id": i, "k": i % 4, "amt": i })
        .collect()
}

async fn plan_of(
    store: &dyn DocumentStore,
    ns: &Namespace,
    texts: &[&str],
    verbosity: ExplainVerbosity,
) -> AggPlan {
    let pipeline = explain_pipeline(&stages(texts), texts.len() - 1).unwrap();
    let explain = store
        .aggregate_explain(ns, pipeline, verbosity, MAX_TIME)
        .await
        .unwrap();
    assert_eq!(
        explain.plan_only,
        verbosity == ExplainVerbosity::QueryPlanner
    );
    match parse_aggregate_explain(&explain.document) {
        AggPlanOutcome::Plan(plan) => plan,
        AggPlanOutcome::NotUnderstood => panic!("not understood: {}", explain.document),
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_plan_only_explain_reads_the_server_plan_without_counters() {
    let name = "a_plan_only_explain_reads_the_server_plan_without_counters";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let texts = [
        r#"{"$match": {"amt": {"$gte": 100}}}"#,
        r#"{"$group": {"_id": "$k", "n": {"$sum": 1}}}"#,
        r#"{"$sort": {"_id": 1}}"#,
    ];
    let plan = plan_of(&store, &ns, &texts, ExplainVerbosity::QueryPlanner).await;
    assert_eq!(plan.summary.verbosity_seen, ExplainVerbosity::QueryPlanner);
    assert_eq!(plan.summary.docs_examined, None);
    let input = plan.input.as_ref().or_else(|| {
        plan.shards
            .first()
            .and_then(|(_, shard)| shard.input.as_ref())
    });
    assert_eq!(input.map(|i| i.stage_kind.as_str()), Some("COLLSCAN"));
    assert!(
        plan.hints.contains(&AggHint::CollectionScan),
        "{:?}",
        plan.hints
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn execution_stats_count_what_the_server_examined_and_an_index_changes_the_plan() {
    let name = "execution_stats_count_what_the_server_examined_and_an_index_changes_the_plan";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let texts = [
        r#"{"$match": {"k": 1}}"#,
        r#"{"$group": {"_id": null, "total": {"$sum": "$amt"}}}"#,
    ];
    let scanned = plan_of(&store, &ns, &texts, ExplainVerbosity::ExecutionStats).await;
    assert_eq!(
        scanned.summary.verbosity_seen,
        ExplainVerbosity::ExecutionStats
    );
    assert_eq!(scanned.summary.docs_examined, Some(400));
    assert!(scanned.hints.contains(&AggHint::CollectionScan));

    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .create_index(IndexModel::builder().keys(doc! { "k": 1 }).build())
        .await
        .unwrap();
    let indexed = plan_of(&store, &ns, &texts, ExplainVerbosity::ExecutionStats).await;
    let input = indexed
        .input
        .as_ref()
        .expect("the input of an indexed match");
    assert_eq!(input.stage_kind, "IXSCAN");
    assert_eq!(input.index.as_deref(), Some("k_1"));
    assert_eq!(indexed.summary.docs_examined, Some(100));
    assert!(!indexed.hints.contains(&AggHint::CollectionScan));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_lookup_and_a_facet_explain_and_parse() {
    let name = "a_lookup_and_a_facet_explain_and_parse";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    insert_labelled(
        &client,
        &dbs.ns("people"),
        &[doc! { "_id": 1, "name": "one" }],
    )
    .await;
    let looked = plan_of(
        &store,
        &ns,
        &[r#"{"$lookup": {"from": "people", "localField": "k", "foreignField": "_id", "as": "who"}}"#],
        ExplainVerbosity::QueryPlanner,
    )
    .await;
    // Depending on the server version the join is a stage of the pipeline or pushed into the plan.
    let pushed_down = looked
        .input
        .as_ref()
        .is_some_and(|i| i.plan_stages.iter().any(|s| s == "EQ_LOOKUP"));
    assert!(
        pushed_down || looked.stages.iter().any(|s| s.name == "$lookup"),
        "{looked:?}"
    );
    let faceted = plan_of(
        &store,
        &ns,
        &[r#"{"$facet": {"n": [{"$count": "n"}], "top": [{"$sort": {"amt": -1}}, {"$limit": 3}]}}"#],
        ExplainVerbosity::ExecutionStats,
    )
    .await;
    assert!(
        faceted.stages.iter().any(|s| s.name == "$facet"),
        "{faceted:?}"
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_write_stage_is_refused_before_the_server_and_a_bad_stage_is_a_server_error() {
    let name = "a_write_stage_is_refused_before_the_server_and_a_bad_stage_is_a_server_error";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let out = store
        .aggregate_explain(
            &ns,
            vec![doc! { "$match": {} }, doc! { "$out": "stolen" }],
            ExplainVerbosity::QueryPlanner,
            MAX_TIME,
        )
        .await;
    assert!(matches!(out, Err(CoreError::InvalidDocument(_))), "{out:?}");
    let nested = doc! { "$facet": { "f": [ { "$merge": "stolen" } ] } };
    let out = store
        .aggregate_explain(
            &ns,
            vec![nested],
            ExplainVerbosity::ExecutionStats,
            MAX_TIME,
        )
        .await;
    assert!(matches!(out, Err(CoreError::InvalidDocument(_))), "{out:?}");
    let names = client
        .database(&ns.db)
        .list_collection_names()
        .await
        .unwrap();
    assert!(!names.iter().any(|n| n == "stolen"), "{names:?}");

    let bad = store
        .aggregate_explain(
            &ns,
            vec![doc! { "$grup": { "_id": 1 } }],
            ExplainVerbosity::QueryPlanner,
            MAX_TIME,
        )
        .await;
    assert!(matches!(bad, Err(CoreError::Database(_))), "{bad:?}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_read_only_connection_can_explain() {
    let name = "a_read_only_connection_can_explain";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let locked = ReadOnlyStore::wrap(Arc::new(store), Arc::new(AtomicBool::new(true)));
    let plan = plan_of(
        &*locked,
        &ns,
        &[r#"{"$match": {"k": 2}}"#],
        ExplainVerbosity::QueryPlanner,
    )
    .await;
    assert!(plan.input.is_some());
}
