//! Tier 1 (local server): aggregate options against a real MongoDB: collation, hint, comment,
//! the lowered time limit, the two safety rules (no disk use on a preview, the primary for a run)
//! and the typed errors. Writes only to throwaway `cbit_*` databases. Run with
//! `scripts/integration-tests.sh`; the read preference tests need a replica set member.

mod support;

use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::client;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{AggPlanOutcome, ExplainVerbosity, parse_aggregate_explain};
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::pipeline::{
    CaseFirst, CollationOption, HintOption, Operation, PipelineOptions, ReadPreferenceMode,
    Strength,
};
use curlybrace_core::store::{DocumentStore, Namespace};
use mongodb::{Client, IndexModel};
use support::{insert_labelled, local_enabled, local_mongo, replica_set_or_skip};

fn collation(locale: &str) -> CollationOption {
    CollationOption {
        locale: locale.to_string(),
        strength: None,
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    }
}

fn with_collation(c: CollationOption) -> PipelineOptions {
    PipelineOptions {
        collation: Some(c),
        ..PipelineOptions::default()
    }
}

fn with_hint(hint: HintOption) -> PipelineOptions {
    PipelineOptions {
        hint: Some(hint),
        ..PipelineOptions::default()
    }
}

async fn preview(
    store: &dyn DocumentStore,
    ns: &Namespace,
    pipeline: Vec<Document>,
    options: &PipelineOptions,
) -> Result<Vec<Document>, CoreError> {
    store
        .aggregate_preview(ns, pipeline, &options.applied(Operation::Preview))
        .await
}

fn names(docs: &[Document]) -> Vec<String> {
    docs.iter()
        .map(|d| d.get_str("name").unwrap_or_default().to_string())
        .collect()
}

async fn index(client: &Client, ns: &Namespace, keys: Document) {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .create_index(IndexModel::builder().keys(keys).build())
        .await
        .unwrap();
}

async fn hint_collection(client: &Client, ns: &Namespace) {
    let docs: Vec<Document> = (0..40)
        .map(|i| doc! { "_id": i, "k": i % 4, "amt": i })
        .collect();
    insert_labelled(client, ns, &docs).await;
    index(client, ns, doc! { "k": 1 }).await;
    index(client, ns, doc! { "amt": 1 }).await;
}

fn hinted_pipeline() -> Vec<Document> {
    vec![doc! { "$match": { "k": 1, "amt": { "$gte": 0 } } }]
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn collation_strength_two_matches_and_sorts_ignoring_case() {
    if !local_enabled("collation_strength_two_matches_and_sorts_ignoring_case") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("words");
    let names_in = ["b", "A", "abc", "C", "ABC", "d"];
    let docs: Vec<Document> = names_in
        .iter()
        .enumerate()
        .map(|(i, n)| doc! { "_id": i as i32, "name": *n })
        .collect();
    insert_labelled(&client, &ns, &docs).await;
    let options = with_collation(CollationOption {
        strength: Some(Strength::Secondary),
        ..collation("en")
    });
    let matched = preview(
        &store,
        &ns,
        vec![doc! { "$match": { "name": "abc" } }],
        &options,
    )
    .await
    .unwrap();
    assert_eq!(matched.len(), 2, "ABC and abc match at strength 2");
    let plain = preview(
        &store,
        &ns,
        vec![doc! { "$match": { "name": "abc" } }],
        &PipelineOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(plain.len(), 1);
    let sorted = preview(
        &store,
        &ns,
        vec![
            doc! { "$match": { "name": { "$in": ["b", "A", "C", "d"] } } },
            doc! { "$sort": { "name": 1 } },
        ],
        &options,
    )
    .await
    .unwrap();
    assert_eq!(names(&sorted), ["A", "b", "C", "d"]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn numeric_ordering_sorts_digit_strings_as_numbers() {
    if !local_enabled("numeric_ordering_sorts_digit_strings_as_numbers") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("items");
    let docs: Vec<Document> = ["item10", "item9", "item2"]
        .iter()
        .enumerate()
        .map(|(i, n)| doc! { "_id": i as i32, "name": *n })
        .collect();
    insert_labelled(&client, &ns, &docs).await;
    let sort = vec![doc! { "$sort": { "name": 1 } }];
    let numeric = with_collation(CollationOption {
        numeric_ordering: Some(true),
        case_first: Some(CaseFirst::Off),
        ..collation("en")
    });
    let sorted = preview(&store, &ns, sort.clone(), &numeric).await.unwrap();
    assert_eq!(names(&sorted), ["item2", "item9", "item10"]);
    let text = preview(&store, &ns, sort, &PipelineOptions::default())
        .await
        .unwrap();
    assert_eq!(names(&text), ["item10", "item2", "item9"]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn an_invalid_locale_is_collation_rejected() {
    if !local_enabled("an_invalid_locale_is_collation_rejected") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("words");
    insert_labelled(&client, &ns, &[doc! { "_id": 1, "name": "a" }]).await;
    let out = preview(
        &store,
        &ns,
        vec![doc! { "$match": {} }],
        &with_collation(collation("qq")),
    )
    .await;
    assert_eq!(out, Err(CoreError::CollationRejected));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_hint_by_name_and_by_keys_is_the_plan_s_index() {
    if !local_enabled("a_hint_by_name_and_by_keys_is_the_plan_s_index") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    hint_collection(&client, &ns).await;
    let cases = [
        (HintOption::Name("amt_1".into()), "amt_1"),
        (HintOption::Keys(doc! { "k": 1 }), "k_1"),
        (HintOption::Keys(doc! { "amt": 1 }), "amt_1"),
    ];
    for (hint, expected) in cases {
        let options = with_hint(hint);
        let explain = store
            .aggregate_explain(
                &ns,
                hinted_pipeline(),
                ExplainVerbosity::QueryPlanner,
                &options.applied(Operation::Explain),
            )
            .await
            .unwrap();
        let AggPlanOutcome::Plan(plan) = parse_aggregate_explain(&explain.document) else {
            panic!("not understood: {}", explain.document);
        };
        let used = plan.input.and_then(|i| i.index);
        assert_eq!(used.as_deref(), Some(expected));
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_hint_on_a_missing_index_is_hint_not_found() {
    if !local_enabled("a_hint_on_a_missing_index_is_hint_not_found") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    hint_collection(&client, &ns).await;
    for hint in [
        HintOption::Name("nope_1".into()),
        HintOption::Keys(doc! { "zzz": 1 }),
    ] {
        let options = with_hint(hint);
        let previewed = preview(&store, &ns, hinted_pipeline(), &options).await;
        assert_eq!(previewed, Err(CoreError::HintNotFound));
        let counted = preview(
            &store,
            &ns,
            vec![
                doc! { "$match": { "k": 1 } },
                doc! { "$limit": 100 },
                doc! { "$count": "n" },
            ],
            &options,
        )
        .await;
        assert_eq!(counted, Err(CoreError::HintNotFound));
        let explained = store
            .aggregate_explain(
                &ns,
                hinted_pipeline(),
                ExplainVerbosity::QueryPlanner,
                &options.applied(Operation::Explain),
            )
            .await;
        assert_eq!(explained.err(), Some(CoreError::HintNotFound));
    }
    // A hint that names a real index still works: the error is not just any BadValue.
    let ok = preview(
        &store,
        &ns,
        hinted_pipeline(),
        &with_hint(HintOption::Name("k_1".into())),
    )
    .await;
    assert!(ok.is_ok(), "{ok:?}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_lowered_max_time_expires() {
    if !local_enabled("a_lowered_max_time_expires") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("big");
    let docs: Vec<Document> = (0..3_000).map(|i| doc! { "_id": i, "v": i }).collect();
    insert_labelled(&client, &ns, &docs).await;
    // An unindexed self cross join, far beyond 50 ms.
    let pipeline = vec![
        doc! { "$lookup": {
            "from": "big",
            "let": { "v": "$v" },
            "pipeline": [ { "$match": { "$expr": { "$gte": ["$v", "$$v"] } } } ],
            "as": "cross",
        } },
        doc! { "$group": { "_id": null, "n": { "$sum": { "$size": "$cross" } } } },
    ];
    let options = PipelineOptions {
        max_time_ms: Some(50),
        ..PipelineOptions::default()
    };
    let out = preview(&store, &ns, pipeline, &options).await;
    assert_eq!(out, Err(CoreError::Database("timeout".into())));
}

/// The newest `system.profile` entry of `ns` for which `matches` holds, polled briefly.
async fn profiled(client: &Client, db: &str, filter: Document) -> Option<Document> {
    let profile = client.database(db).collection::<Document>("system.profile");
    for _ in 0..20 {
        if let Some(entry) = profile.find_one(filter.clone()).await.unwrap() {
            return Some(entry);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

async fn set_profiling(client: &Client, db: &str, level: i32) {
    client
        .database(db)
        .run_command(doc! { "profile": level })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn the_comment_reaches_the_profiler() {
    if !local_enabled("the_comment_reaches_the_profiler") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &[doc! { "_id": 1 }]).await;
    set_profiling(&client, &ns.db, 2).await;
    let options = PipelineOptions {
        comment: Some("curlybrace-it-comment".into()),
        ..PipelineOptions::default()
    };
    preview(&store, &ns, vec![doc! { "$match": {} }], &options)
        .await
        .unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.aggregate": "orders", "command.comment": "curlybrace-it-comment" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    assert!(entry.is_some(), "no profiled aggregate carried the comment");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn disk_use_is_false_on_preview_even_when_set() {
    if !local_enabled("disk_use_is_false_on_preview_even_when_set") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    let docs: Vec<Document> = (0..200).map(|i| doc! { "_id": i, "k": i % 7 }).collect();
    insert_labelled(&client, &ns, &docs).await;
    set_profiling(&client, &ns.db, 2).await;
    let options = PipelineOptions {
        allow_disk_use: Some(true),
        comment: Some("curlybrace-it-disk".into()),
        ..PipelineOptions::default()
    };
    preview(
        &store,
        &ns,
        vec![doc! { "$group": { "_id": "$k", "n": { "$sum": 1 } } }],
        &options,
    )
    .await
    .unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.aggregate": "orders", "command.comment": "curlybrace-it-disk" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    let command = entry
        .expect("the profiled preview")
        .get_document("command")
        .cloned()
        .unwrap();
    assert_eq!(command.get_bool("allowDiskUse"), Ok(false));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_run_with_collation_and_hint_writes_the_expected_target() {
    if !local_enabled("a_run_with_collation_and_hint_writes_the_expected_target") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let source = dbs.ns("words");
    let docs: Vec<Document> = ["abc", "ABC", "abd"]
        .iter()
        .enumerate()
        .map(|(i, n)| doc! { "_id": i as i32, "name": *n })
        .collect();
    insert_labelled(&client, &source, &docs).await;
    index(&client, &source, doc! { "name": 1 }).await;
    let options = PipelineOptions {
        collation: Some(CollationOption {
            strength: Some(Strength::Secondary),
            ..collation("en")
        }),
        hint: Some(HintOption::Name("name_1".into())),
        max_time_ms: Some(30_000),
        allow_disk_use: Some(true),
        comment: Some("curlybrace-it-run".into()),
        read_preference: Some(ReadPreferenceMode::Secondary),
    };
    let pipeline = vec![
        doc! { "$match": { "name": "abc" } },
        doc! { "$out": "target" },
    ];
    store
        .aggregate_write(&source, pipeline, &options.applied(Operation::Run))
        .await
        .unwrap();
    let target = dbs.ns("target");
    let written = preview(
        &store,
        &target,
        vec![doc! { "$sort": { "_id": 1 } }],
        &PipelineOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(names(&written), ["abc", "ABC"]);
}

/// `uri` with `params` appended as query options.
fn with_params(uri: &str, params: &str) -> String {
    match (uri.contains('?'), uri.ends_with('/') || uri.contains("/?")) {
        (true, _) => format!("{uri}&{params}"),
        (false, true) => format!("{uri}?{params}"),
        (false, false) => format!("{uri}/?{params}"),
    }
}

async fn store_for(uri: &str) -> (Client, MongoStore) {
    let client = client::connect(&ConnectionConfig::new("it", uri), None, None)
        .await
        .unwrap();
    let store = MongoStore::new(client.clone());
    (client, store)
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn secondary_preferred_reads_on_a_one_member_set() {
    let name = "secondary_preferred_reads_on_a_one_member_set";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    if !replica_set_or_skip(&client, name).await {
        return;
    }
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &[doc! { "_id": 1 }]).await;
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
        ..PipelineOptions::default()
    };
    let out = preview(&store, &ns, vec![doc! { "$match": {} }], &options)
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn secondary_on_a_one_member_set_is_read_preference_unavailable() {
    let name = "secondary_on_a_one_member_set_is_read_preference_unavailable";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, _) = local_mongo().await;
    if !replica_set_or_skip(&client, name).await {
        return;
    }
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &[doc! { "_id": 1 }]).await;
    let uri = with_params(
        &curlybrace_testkit::local_uri(),
        "serverSelectionTimeoutMS=2000",
    );
    let (_, short) = store_for(&uri).await;
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Secondary),
        ..PipelineOptions::default()
    };
    let out = preview(&short, &ns, vec![doc! { "$match": {} }], &options).await;
    assert_eq!(out, Err(CoreError::ReadPreferenceUnavailable));
    let explained = short
        .aggregate_explain(
            &ns,
            vec![doc! { "$match": {} }],
            ExplainVerbosity::QueryPlanner,
            &options.applied(Operation::Explain),
        )
        .await;
    assert_eq!(explained.err(), Some(CoreError::ReadPreferenceUnavailable));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_run_pins_the_primary_whatever_the_connection_and_the_options_say() {
    let name = "a_run_pins_the_primary_whatever_the_connection_and_the_options_say";
    if !local_enabled(name) {
        return;
    }
    let (dbs, client, _) = local_mongo().await;
    if !replica_set_or_skip(&client, name).await {
        return;
    }
    let source = dbs.ns("orders");
    insert_labelled(&client, &source, &[doc! { "_id": 1, "k": 1 }]).await;
    let uri = with_params(
        &curlybrace_testkit::local_uri(),
        "readPreference=secondary&serverSelectionTimeoutMS=2000",
    );
    let (_, secondary_store) = store_for(&uri).await;
    // Reading on this connection finds no secondary ...
    let read = secondary_store
        .aggregate_preview(
            &source,
            vec![doc! { "$match": {} }],
            &PipelineOptions::default().applied(Operation::Preview),
        )
        .await;
    assert!(read.is_err(), "{read:?}");
    // ... yet the confirmed run goes to the primary and writes.
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Secondary),
        ..PipelineOptions::default()
    };
    secondary_store
        .aggregate_write(
            &source,
            vec![doc! { "$match": {} }, doc! { "$out": "copy" }],
            &options.applied(Operation::Run),
        )
        .await
        .unwrap();
    let copied = client
        .database(&source.db)
        .collection::<Document>("copy")
        .count_documents(doc! {})
        .await
        .unwrap();
    assert_eq!(copied, 1);
}
