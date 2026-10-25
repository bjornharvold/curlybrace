//! Collection options (capped, validator, collation) written and read back on a real server.
//! Tier 2 (docker): own `cb-it-*` container per test; run with `scripts/container-tests.sh`.

mod support;

use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{CappedSpec, CollectionOptions, DocumentStore, Namespace};
use support::{docker_enabled, docker_mongo};

const DB: &str = "cbit_copy_options";

fn ns(collection: &str) -> Namespace {
    Namespace::new(DB, collection)
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn a_capped_validated_collated_collection_reads_back_equal() {
    if !docker_enabled("a_capped_validated_collated_collection_reads_back_equal") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let wanted = CollectionOptions {
        capped: Some(CappedSpec {
            size: 65_536,
            max: Some(100),
        }),
        validator: Some(doc! { "a": { "$exists": true } }),
        validation_level: Some("moderate".into()),
        validation_action: Some("warn".into()),
        collation: Some(doc! { "locale": "fr", "strength": 2 }),
    };
    store.create_collection(&ns("rich"), &wanted).await.unwrap();
    let got = store
        .collection_options(&ns("rich"))
        .await
        .unwrap()
        .unwrap();
    // The server fills in the collation's other fields, so compare what was asked for.
    let collation = got.collation.clone().unwrap();
    assert_eq!(collation.get_str("locale"), Ok("fr"));
    assert_eq!(collation.get_i32("strength"), Ok(2));
    assert_eq!(
        CollectionOptions {
            collation: wanted.collation.clone(),
            ..got
        },
        wanted
    );
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn a_plain_collection_has_default_options_and_a_missing_one_has_none() {
    if !docker_enabled("a_plain_collection_has_default_options_and_a_missing_one_has_none") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    store
        .create_collection(&ns("plain"), &CollectionOptions::default())
        .await
        .unwrap();
    assert_eq!(
        store.collection_options(&ns("plain")).await.unwrap(),
        Some(CollectionOptions::default())
    );
    assert_eq!(store.collection_options(&ns("absent")).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn creating_an_existing_collection_fails() {
    if !docker_enabled("creating_an_existing_collection_fails") {
        return;
    }
    let (_c, _client, store) = docker_mongo().await;
    let capped = CollectionOptions {
        capped: Some(CappedSpec {
            size: 65_536,
            max: None,
        }),
        ..CollectionOptions::default()
    };
    let plain = CollectionOptions::default();
    store.create_collection(&ns("twice"), &plain).await.unwrap();
    // The server treats a repeat with identical options as a no-op, so ask for different ones.
    let again: Result<(), CoreError> = store.create_collection(&ns("twice"), &capped).await;
    assert!(again.is_err());
    assert_eq!(
        store.collection_options(&ns("twice")).await.unwrap(),
        Some(plain),
        "the first options stay"
    );
}
