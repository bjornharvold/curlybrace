//! Tier 1 (local server): per-stage counts against a real MongoDB. Writes only to throwaway
//! `cbit_*` databases. Run with `scripts/integration-tests.sh`, which points at a server you
//! name through `CURLYBRACE_IT_MONGODB_URI`.

mod support;

use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{
    AppliedOptions, COUNT_CAP, Operation, PREVIEW_MAX_TIME_MS, Stage, count_of, count_pipeline,
};
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use curlybrace_core::store::{DocumentStore, Namespace};
use support::{insert_labelled, local_enabled, local_mongo};

const MAX_TIME: Duration = Duration::from_millis(PREVIEW_MAX_TIME_MS);

fn bounded(d: Duration) -> AppliedOptions {
    AppliedOptions::bounded(Operation::Preview, d)
}

fn stages(texts: &[&str]) -> Vec<Stage> {
    texts
        .iter()
        .map(|t| Stage {
            text: (*t).to_string(),
            enabled: true,
        })
        .collect()
}

/// 400 orders: `k` cycles 0..4, `amt` is the index.
fn orders() -> Vec<Document> {
    (0..400)
        .map(|i| doc! { "_id": i, "k": i % 4, "amt": i })
        .collect()
}

async fn count(
    store: &dyn DocumentStore,
    ns: &Namespace,
    texts: &[&str],
    upto: usize,
    cap: i64,
    max_time: Duration,
) -> curlybrace_core::error::Result<u64> {
    let pipeline = count_pipeline(&stages(texts), upto, cap).unwrap();
    count_of(
        &store
            .aggregate_preview(ns, pipeline, &bounded(max_time))
            .await?,
    )
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn each_stage_counts_the_documents_it_lets_through() {
    let name = "each_stage_counts_the_documents_it_lets_through";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let texts = [
        r#"{"$match": {"amt": {"$gte": 100}}}"#,
        r#"{"$group": {"_id": "$k", "n": {"$sum": 1}}}"#,
        r#"{"$limit": 3}"#,
    ];
    let counts = [
        count(&store, &ns, &texts, 0, COUNT_CAP, MAX_TIME).await,
        count(&store, &ns, &texts, 1, COUNT_CAP, MAX_TIME).await,
        count(&store, &ns, &texts, 2, COUNT_CAP, MAX_TIME).await,
    ];
    assert_eq!(counts, [Ok(300), Ok(4), Ok(3)]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn the_cap_stops_the_count_and_an_empty_result_is_zero() {
    let name = "the_cap_stops_the_count_and_an_empty_result_is_zero";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    let all = [r#"{"$match": {}}"#];
    // The collection is bigger than the (test) cap: the answer is the cap, which the UI shows as
    // "at least".
    assert_eq!(count(&store, &ns, &all, 0, 25, MAX_TIME).await, Ok(25));
    assert_eq!(count(&store, &ns, &all, 0, 400, MAX_TIME).await, Ok(400));
    let none = [r#"{"$match": {"amt": -1}}"#];
    assert_eq!(
        count(&store, &ns, &none, 0, COUNT_CAP, MAX_TIME).await,
        Ok(0)
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_count_that_outruns_its_time_limit_is_an_error_not_a_number() {
    let name = "a_count_that_outruns_its_time_limit_is_an_error_not_a_number";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &orders()).await;
    // Slow without $where (refused inside an aggregation): a busy loop per document.
    let slow = [
        r#"{"$match": {"$expr": {"$function": {"body": "function() { var t = Date.now(); while (Date.now() - t < 20) {} return true; }", "args": [], "lang": "js"}}}}"#,
    ];
    let outcome = count(&store, &ns, &slow, 0, COUNT_CAP, Duration::from_millis(50)).await;
    // The server's own time limit (MaxTimeMSExpired), not some other failure such as $where being
    // disabled: that is what the handler maps to the card's "timed out".
    assert_eq!(
        outcome,
        Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        "{outcome:?}"
    );
}
