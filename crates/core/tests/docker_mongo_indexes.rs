//! Index listing, creation, hiding, TTL changes, dropping and usage stats against a real server.
//! Tier 2 (docker): own `cb-it-*` container per test; run with `scripts/container-tests.sh`.

mod support;

use bson::{Document, doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::indexes::{IndexOptions, IndexSpec, KeyKind};
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::store::{CollectionIndex, DocumentStore, Namespace};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use support::{docker_enabled, docker_mongo};

const DB: &str = "cbit_indexes";

fn ns() -> Namespace {
    Namespace::new(DB, "items")
}

/// An index called `name` over `keys` with `options`.
fn spec(name: &str, keys: &[(&str, KeyKind)], options: IndexOptions) -> IndexSpec {
    IndexSpec {
        options,
        ..mock_index_spec_named(name, keys)
    }
}

/// Creates `spec` on `items` and returns what `list_indexes` reports under its name.
async fn create_and_list(store: &MongoStore, spec: &IndexSpec) -> CollectionIndex {
    store.create_index(&ns(), spec).await.unwrap();
    find_index(store, &spec.name).await
}

async fn find_index(store: &MongoStore, name: &str) -> CollectionIndex {
    let listed = store.list_indexes(&ns()).await.unwrap();
    listed
        .into_iter()
        .find(|i| i.spec.name == name)
        .unwrap_or_else(|| panic!("index {name} is not listed"))
}

/// Seeds `items` so index sizes are non-zero and the collection exists.
async fn seed(store: &MongoStore, count: i32) {
    for i in 0..count {
        store
            .insert(
                &ns(),
                doc! { "_id": i, "a": i, "b": format!("v{i}"), "t": format!("word{i} text"),
                "loc": { "type": "Point", "coordinates": [f64::from(i % 90), 10.0] } },
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_single_field_index() {
    if !docker_enabled("creates_and_lists_a_single_field_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec("a_1", &[("a", KeyKind::Asc)], IndexOptions::default());
    let got = create_and_list(&store, &wanted).await;
    assert_eq!(got.spec.name, "a_1");
    assert_eq!(got.spec.keys, wanted.keys);
    assert_eq!(got.spec.options, IndexOptions::default());
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_compound_index() {
    if !docker_enabled("creates_and_lists_a_compound_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec(
        "a_1_b_-1",
        &[("a", KeyKind::Asc), ("b", KeyKind::Desc)],
        IndexOptions::default(),
    );
    let got = create_and_list(&store, &wanted).await;
    assert_eq!(got.spec.keys, wanted.keys);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_text_index() {
    if !docker_enabled("creates_and_lists_a_text_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec(
        "t_text",
        &[("t", KeyKind::Text), ("b", KeyKind::Text)],
        IndexOptions::default(),
    );
    let got = create_and_list(&store, &wanted).await;
    let mut fields: Vec<_> = got.spec.keys.iter().map(|k| k.field.as_str()).collect();
    fields.sort_unstable();
    assert_eq!(fields, ["b", "t"]);
    assert!(got.spec.keys.iter().all(|k| k.kind == KeyKind::Text));
    // The server's weights survive in the raw document, so a copy keeps them.
    assert!(got.spec.raw.get_document("weights").is_ok());
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_2dsphere_index() {
    if !docker_enabled("creates_and_lists_a_2dsphere_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec(
        "loc_2dsphere",
        &[("loc", KeyKind::Geo2dSphere)],
        IndexOptions::default(),
    );
    let got = create_and_list(&store, &wanted).await;
    assert_eq!(got.spec.keys, wanted.keys);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_wildcard_index_with_projection() {
    if !docker_enabled("creates_and_lists_a_wildcard_index_with_projection") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let projection = doc! { "a": 1_i32, "b": 1_i32 };
    let wanted = spec(
        "wild",
        &[("$**", KeyKind::Wildcard)],
        IndexOptions {
            wildcard_projection: Some(projection.clone()),
            ..IndexOptions::default()
        },
    );
    let got = create_and_list(&store, &wanted).await;
    assert_eq!(got.spec.keys, wanted.keys);
    assert_eq!(got.spec.options.wildcard_projection, Some(projection));
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_unique_index() {
    if !docker_enabled("creates_and_lists_a_unique_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec(
        "b_1",
        &[("b", KeyKind::Asc)],
        IndexOptions {
            unique: true,
            ..IndexOptions::default()
        },
    );
    let got = create_and_list(&store, &wanted).await;
    assert!(got.spec.options.unique);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_ttl_index() {
    if !docker_enabled("creates_and_lists_a_ttl_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec(
        "created_1",
        &[("created", KeyKind::Asc)],
        IndexOptions {
            expire_after_seconds: Some(3600),
            ..IndexOptions::default()
        },
    );
    let got = create_and_list(&store, &wanted).await;
    assert_eq!(got.spec.options.expire_after_seconds, Some(3600));
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_partial_index() {
    if !docker_enabled("creates_and_lists_a_partial_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let filter = doc! { "a": { "$gt": 5_i32 } };
    let wanted = spec(
        "a_partial",
        &[("a", KeyKind::Asc)],
        IndexOptions {
            partial_filter: Some(filter.clone()),
            ..IndexOptions::default()
        },
    );
    let got = create_and_list(&store, &wanted).await;
    assert_eq!(got.spec.options.partial_filter, Some(filter));
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creates_and_lists_a_collation_index() {
    if !docker_enabled("creates_and_lists_a_collation_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec(
        "b_collated",
        &[("b", KeyKind::Asc)],
        IndexOptions {
            collation: Some(doc! { "locale": "en", "strength": 2_i32 }),
            ..IndexOptions::default()
        },
    );
    let got = create_and_list(&store, &wanted).await;
    let collation = got.spec.options.collation.expect("collation is listed");
    assert_eq!(collation.get_str("locale"), Ok("en"));
    assert_eq!(collation.get_i32("strength"), Ok(2));
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn hide_sets_hidden_in_list_indexes() {
    if !docker_enabled("hide_sets_hidden_in_list_indexes") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec("a_1", &[("a", KeyKind::Asc)], IndexOptions::default());
    create_and_list(&store, &wanted).await;
    store.set_index_hidden(&ns(), "a_1", true).await.unwrap();
    assert!(find_index(&store, "a_1").await.spec.options.hidden);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn unhide_clears_it() {
    if !docker_enabled("unhide_clears_it") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec("a_1", &[("a", KeyKind::Asc)], IndexOptions::default());
    create_and_list(&store, &wanted).await;
    store.set_index_hidden(&ns(), "a_1", true).await.unwrap();
    store.set_index_hidden(&ns(), "a_1", false).await.unwrap();
    assert!(!find_index(&store, "a_1").await.spec.options.hidden);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn set_ttl_changes_expire_after_seconds_via_collmod() {
    if !docker_enabled("set_ttl_changes_expire_after_seconds_via_collmod") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec(
        "created_1",
        &[("created", KeyKind::Asc)],
        IndexOptions {
            expire_after_seconds: Some(60),
            ..IndexOptions::default()
        },
    );
    create_and_list(&store, &wanted).await;
    store.set_index_ttl(&ns(), "created_1", 7200).await.unwrap();
    let got = find_index(&store, "created_1").await;
    assert_eq!(got.spec.options.expire_after_seconds, Some(7200));
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn drop_removes_the_index() {
    if !docker_enabled("drop_removes_the_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = spec("a_1", &[("a", KeyKind::Asc)], IndexOptions::default());
    create_and_list(&store, &wanted).await;
    store.drop_index(&ns(), "a_1").await.unwrap();
    let names: Vec<_> = store
        .list_indexes(&ns())
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.spec.name)
        .collect();
    assert_eq!(names, ["_id_"]);
    // A second drop finds nothing to drop.
    assert_eq!(
        store.drop_index(&ns(), "a_1").await,
        Err(CoreError::NotFound)
    );
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn dropping_id_is_refused_by_the_server_and_reported() {
    if !docker_enabled("dropping_id_is_refused_by_the_server_and_reported") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    seed(&store, 1).await;
    let err = store.drop_index(&ns(), "_id_").await.unwrap_err();
    assert!(
        matches!(&err, CoreError::Database(text) if text.contains("_id")),
        "{err:?}"
    );
    // The index is still there.
    find_index(&store, "_id_").await;
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn index_stats_reports_ops_that_grow_after_a_query_uses_the_index() {
    if !docker_enabled("index_stats_reports_ops_that_grow_after_a_query_uses_the_index") {
        return;
    }
    let (_c, client, store) = docker_mongo().await;
    seed(&store, 20).await;
    let wanted = spec("a_1", &[("a", KeyKind::Asc)], IndexOptions::default());
    store.create_index(&ns(), &wanted).await.unwrap();

    let ops = |rows: &[curlybrace_core::store::IndexUsage]| -> i64 {
        rows.iter().filter(|r| r.name == "a_1").map(|r| r.ops).sum()
    };
    let before = store.index_stats(&ns()).await.unwrap();
    let row = before.iter().find(|r| r.name == "a_1").expect("a_1 row");
    assert!(!row.host.is_empty());
    assert!(row.since_ms > 0);
    assert_eq!(row.spec.keys, wanted.keys);
    let before_ops = ops(&before);

    let items = client.database(DB).collection::<Document>("items");
    for _ in 0..3 {
        let _ = futures::TryStreamExt::try_collect::<Vec<_>>(
            items
                .find(doc! { "a": 5_i32 })
                .hint(mongodb::options::Hint::Name("a_1".into()))
                .await
                .unwrap(),
        )
        .await
        .unwrap();
    }
    let after_ops = ops(&store.index_stats(&ns()).await.unwrap());
    assert!(after_ops >= before_ops + 3, "{before_ops} -> {after_ops}");
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn list_indexes_reports_a_size_for_every_index() {
    if !docker_enabled("list_indexes_reports_a_size_for_every_index") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    seed(&store, 50).await;
    let wanted = spec("a_1", &[("a", KeyKind::Asc)], IndexOptions::default());
    store.create_index(&ns(), &wanted).await.unwrap();
    let listed = store.list_indexes(&ns()).await.unwrap();
    assert_eq!(listed.len(), 2);
    for index in &listed {
        assert!(index.size_bytes > 0, "{} has no size", index.spec.name);
    }
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn listing_a_missing_collection_is_empty() {
    if !docker_enabled("listing_a_missing_collection_is_empty") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    assert_eq!(store.list_indexes(&ns()).await, Ok(Vec::new()));
}
