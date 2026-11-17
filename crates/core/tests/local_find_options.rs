//! Tier 1 (local server): the options of a Browse find against a real MongoDB: collation, hint,
//! the lowered time limit, comment, disk use, the default sending no `maxTimeMS`, and the typed
//! errors. Writes only to throwaway `cbit_*` databases. Run against a throwaway `mongod` (never a
//! server that holds data); the read preference tests need a one-member replica set.

mod support;

use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::client;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::pipeline::{
    CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode, Strength,
};
use curlybrace_core::store::{DocumentStore, Namespace, Query};
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

fn query(filter: Document, options: PipelineOptions) -> Query {
    Query {
        filter,
        limit: 0,
        options,
        ..Query::default()
    }
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

async fn words(client: &Client, ns: &Namespace, list: &[&str]) {
    let docs: Vec<Document> = list
        .iter()
        .enumerate()
        .map(|(i, n)| doc! { "_id": i as i32, "name": *n })
        .collect();
    insert_labelled(client, ns, &docs).await;
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn collation_strength_two_matches_and_sorts_ignoring_case() {
    if !local_enabled("collation_strength_two_matches_and_sorts_ignoring_case") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("words");
    words(&client, &ns, &["b", "A", "abc", "C", "ABC", "d"]).await;
    let options = PipelineOptions {
        collation: Some(CollationOption {
            strength: Some(Strength::Secondary),
            ..collation("en")
        }),
        ..PipelineOptions::default()
    };
    let matched = store
        .find(&ns, &query(doc! { "name": "abc" }, options.clone()))
        .await
        .unwrap();
    assert_eq!(matched.docs.len(), 2, "ABC and abc match at strength 2");
    let plain = store
        .find(
            &ns,
            &query(doc! { "name": "abc" }, PipelineOptions::default()),
        )
        .await
        .unwrap();
    assert_eq!(plain.docs.len(), 1, "binary comparison matches one");
    let sorted = Query {
        sort: Some(doc! { "name": 1 }),
        ..query(doc! {}, options)
    };
    let collated = store.find(&ns, &sorted).await.unwrap();
    assert_eq!(names(&collated.docs), ["A", "abc", "ABC", "b", "C", "d"]);
    let binary = Query {
        sort: Some(doc! { "name": 1 }),
        ..query(doc! {}, PipelineOptions::default())
    };
    let plain = store.find(&ns, &binary).await.unwrap();
    assert_eq!(names(&plain.docs), ["A", "ABC", "C", "abc", "b", "d"]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn numeric_ordering_sorts_digit_strings_as_numbers() {
    if !local_enabled("numeric_ordering_sorts_digit_strings_as_numbers") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("words");
    words(&client, &ns, &["10", "9", "100", "1"]).await;
    let options = PipelineOptions {
        collation: Some(CollationOption {
            numeric_ordering: Some(true),
            case_first: Some(CaseFirst::Off),
            ..collation("en")
        }),
        ..PipelineOptions::default()
    };
    let q = Query {
        sort: Some(doc! { "name": 1 }),
        ..query(doc! {}, options)
    };
    let page = store.find(&ns, &q).await.unwrap();
    assert_eq!(names(&page.docs), ["1", "9", "10", "100"]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn an_invalid_locale_is_collation_rejected() {
    if !local_enabled("an_invalid_locale_is_collation_rejected") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("words");
    words(&client, &ns, &["a"]).await;
    let options = PipelineOptions {
        collation: Some(collation("zz")),
        ..PipelineOptions::default()
    };
    let found = store.find(&ns, &query(doc! {}, options.clone())).await;
    assert_eq!(found.err(), Some(CoreError::CollationRejected));
    let explained = store.explain_find(&ns, &query(doc! {}, options)).await;
    assert_eq!(explained.err(), Some(CoreError::CollationRejected));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_hint_on_a_missing_index_is_hint_not_found() {
    if !local_enabled("a_hint_on_a_missing_index_is_hint_not_found") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &[doc! { "_id": 1, "k": 1 }]).await;
    let q = query(
        doc! { "k": 1 },
        PipelineOptions {
            hint: Some(HintOption::Name("no_such_index".into())),
            ..PipelineOptions::default()
        },
    );
    assert_eq!(
        store.find(&ns, &q).await.err(),
        Some(CoreError::HintNotFound)
    );
    assert_eq!(
        store.explain_find(&ns, &q).await.err(),
        Some(CoreError::HintNotFound)
    );
    assert_eq!(
        store.explain_plan(&ns, &q).await.err(),
        Some(CoreError::HintNotFound)
    );
}

/// The index the winning plan of an explain reads, if it has one.
fn index_of(explain: &Document) -> Option<String> {
    fn walk(d: &Document) -> Option<String> {
        if let Ok(name) = d.get_str("indexName") {
            return Some(name.to_string());
        }
        d.values().find_map(|v| match v {
            bson::Bson::Document(inner) => walk(inner),
            bson::Bson::Array(items) => items.iter().find_map(|i| match i {
                bson::Bson::Document(inner) => walk(inner),
                _ => None,
            }),
            _ => None,
        })
    }
    walk(
        explain
            .get_document("queryPlanner")
            .ok()?
            .get_document("winningPlan")
            .ok()?,
    )
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_hint_by_name_and_by_keys_is_the_plan_s_index() {
    if !local_enabled("a_hint_by_name_and_by_keys_is_the_plan_s_index") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    let docs: Vec<Document> = (0..40)
        .map(|i| doc! { "_id": i, "k": i % 4, "amt": i })
        .collect();
    insert_labelled(&client, &ns, &docs).await;
    index(&client, &ns, doc! { "k": 1 }).await;
    index(&client, &ns, doc! { "amt": 1 }).await;
    let filter = doc! { "k": 1, "amt": { "$gte": 0 } };
    let by_name = query(
        filter.clone(),
        PipelineOptions {
            hint: Some(HintOption::Name("amt_1".into())),
            ..PipelineOptions::default()
        },
    );
    let by_keys = query(
        filter,
        PipelineOptions {
            hint: Some(HintOption::Keys(doc! { "amt": 1 })),
            ..PipelineOptions::default()
        },
    );
    for q in [&by_name, &by_keys] {
        let plan = store.explain_plan(&ns, q).await.unwrap();
        assert_eq!(index_of(&plan.document).as_deref(), Some("amt_1"));
        let stats = store.explain_find(&ns, q).await.unwrap();
        assert_eq!(index_of(&stats.document).as_deref(), Some("amt_1"));
        assert_eq!(store.find(&ns, q).await.unwrap().docs.len(), 10);
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_lowered_max_time_expires() {
    if !local_enabled("a_lowered_max_time_expires") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    let docs: Vec<Document> = (0..200).map(|i| doc! { "_id": i, "v": i }).collect();
    insert_labelled(&client, &ns, &docs).await;
    let options = PipelineOptions {
        max_time_ms: Some(50),
        ..PipelineOptions::default()
    };
    // 200 documents at 5 ms each is a second of server-side JavaScript against a 50 ms limit.
    let slow = doc! { "$where": "function() { var t = Date.now(); while (Date.now() - t < 5) {} return true; }" };
    let found = store.find(&ns, &query(slow, options)).await;
    assert_eq!(found.err(), Some(CoreError::Database("timeout".into())));
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
        comment: Some("curlybrace-it-find-comment".into()),
        ..PipelineOptions::default()
    };
    store.find(&ns, &query(doc! {}, options)).await.unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.find": "orders", "command.comment": "curlybrace-it-find-comment" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    assert!(entry.is_some(), "no profiled find carried the comment");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn allow_disk_use_reaches_the_server() {
    if !local_enabled("allow_disk_use_reaches_the_server") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    let docs: Vec<Document> = (0..50).map(|i| doc! { "_id": i, "k": i % 7 }).collect();
    insert_labelled(&client, &ns, &docs).await;
    set_profiling(&client, &ns.db, 2).await;
    let options = PipelineOptions {
        allow_disk_use: Some(true),
        comment: Some("curlybrace-it-find-disk".into()),
        ..PipelineOptions::default()
    };
    let q = Query {
        sort: Some(doc! { "k": 1 }),
        ..query(doc! {}, options)
    };
    store.find(&ns, &q).await.unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.find": "orders", "command.comment": "curlybrace-it-find-disk" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    let command = entry
        .expect("the profiled find")
        .get_document("command")
        .cloned();
    assert_eq!(command.unwrap().get_bool("allowDiskUse"), Ok(true));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn the_default_find_sends_no_max_time() {
    if !local_enabled("the_default_find_sends_no_max_time") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("orders");
    insert_labelled(&client, &ns, &[doc! { "_id": 1, "tag": "plain-find" }]).await;
    set_profiling(&client, &ns.db, 2).await;
    store
        .find(
            &ns,
            &query(doc! { "tag": "plain-find" }, PipelineOptions::default()),
        )
        .await
        .unwrap();
    let entry = profiled(
        &client,
        &ns.db,
        doc! { "command.find": "orders", "command.filter.tag": "plain-find" },
    )
    .await;
    set_profiling(&client, &ns.db, 0).await;
    let command = entry
        .expect("the profiled find")
        .get_document("command")
        .cloned()
        .unwrap();
    assert!(!command.contains_key("maxTimeMS"), "{command:?}");
    assert!(!command.contains_key("allowDiskUse"), "{command:?}");
    assert!(!command.contains_key("comment"), "{command:?}");
}

/// `uri` with `params` appended as query options.
fn with_params(uri: &str, params: &str) -> String {
    match (uri.contains('?'), uri.ends_with('/') || uri.contains("/?")) {
        (true, _) => format!("{uri}&{params}"),
        (false, true) => format!("{uri}?{params}"),
        (false, false) => format!("{uri}/?{params}"),
    }
}

async fn store_for(uri: &str) -> MongoStore {
    let client = client::connect(&ConnectionConfig::new("it", uri), None, None)
        .await
        .unwrap();
    MongoStore::new(client)
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
    let page = store
        .find(&ns, &query(doc! {}, options.clone()))
        .await
        .unwrap();
    assert_eq!(page.docs.len(), 1);
    let plan = store
        .explain_plan(&ns, &query(doc! {}, options))
        .await
        .unwrap();
    assert!(plan.plan_only);
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
    let short = store_for(&uri).await;
    let q = query(
        doc! {},
        PipelineOptions {
            read_preference: Some(ReadPreferenceMode::Secondary),
            ..PipelineOptions::default()
        },
    );
    assert_eq!(
        short.find(&ns, &q).await.err(),
        Some(CoreError::ReadPreferenceUnavailable)
    );
    assert_eq!(
        short.explain_find(&ns, &q).await.err(),
        Some(CoreError::ReadPreferenceUnavailable)
    );
}
