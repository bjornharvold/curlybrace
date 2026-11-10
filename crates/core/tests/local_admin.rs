//! Tier 1 (local server): database and collection administration against a real MongoDB.
//! Writes only to throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bson::{Document, doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::read_only_store::ReadOnlyStore;
use curlybrace_core::store::{CollectionKind, CollectionOptions, DocumentStore, Namespace};
use support::{local_enabled, local_mongo};

async fn seed(client: &mongodb::Client, ns: &Namespace, count: i32) {
    let docs: Vec<Document> = (0..count).map(|n| doc! { "_id": n, "n": n }).collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
}

async fn collections(store: &dyn DocumentStore, db: &str) -> Vec<String> {
    let mut names = store.list_collections(db).await.unwrap();
    names.sort();
    names
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn drops_a_collection_and_treats_an_already_missing_one_as_done() {
    if !local_enabled("drops_a_collection_and_treats_an_already_missing_one_as_done") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("doomed");
    seed(&client, &ns, 3).await;
    store.drop_collection(&ns).await.unwrap();
    assert!(
        !collections(&store, &dbs.db)
            .await
            .contains(&"doomed".into())
    );
    assert_eq!(store.drop_collection(&ns).await, Ok(()));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn drops_a_database_but_never_a_server_owned_one() {
    if !local_enabled("drops_a_database_but_never_a_server_owned_one") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    seed(&client, &dbs.ns("c"), 1).await;
    store.drop_database(&dbs.db).await.unwrap();
    assert!(!store.list_databases().await.unwrap().contains(&dbs.db));
    for reserved in ["admin", "local", "config"] {
        assert_eq!(
            store.drop_database(reserved).await,
            Err(CoreError::ReservedDatabase)
        );
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn renames_a_collection_and_honours_the_drop_target_choice() {
    if !local_enabled("renames_a_collection_and_honours_the_drop_target_choice") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    seed(&client, &dbs.ns("a"), 2).await;
    seed(&client, &dbs.ns("b"), 5).await;
    assert_eq!(
        store.rename_collection(&dbs.ns("a"), "b", false).await,
        Err(CoreError::NamespaceExists)
    );
    assert_eq!(store.estimated_count(&dbs.ns("b")).await.unwrap(), 5);
    store
        .rename_collection(&dbs.ns("a"), "b", true)
        .await
        .unwrap();
    assert_eq!(collections(&store, &dbs.db).await, vec!["b".to_string()]);
    assert_eq!(store.estimated_count(&dbs.ns("b")).await.unwrap(), 2);
    assert_eq!(
        store
            .rename_collection(&dbs.ns("b"), "bad$name", false)
            .await,
        Err(CoreError::InvalidNamespace)
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn creates_a_read_only_view_over_a_pipeline() {
    if !local_enabled("creates_a_read_only_view_over_a_pipeline") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    seed(&client, &dbs.ns("src"), 10).await;
    let pipeline = [doc! { "$match": { "n": { "$gte": 7 } } }];
    store
        .create_view(&dbs.ns("big"), "src", &pipeline)
        .await
        .unwrap();
    let kinds = store.list_collection_kinds(&dbs.db).await.unwrap();
    assert!(kinds.contains(&("big".to_string(), CollectionKind::View)));
    let found = client
        .database(&dbs.db)
        .collection::<Document>("big")
        .count_documents(doc! {})
        .await
        .unwrap();
    assert_eq!(found, 3);
    assert!(
        store
            .insert(&dbs.ns("big"), doc! { "_id": 99 })
            .await
            .is_err()
    );
    store.drop_collection(&dbs.ns("big")).await.unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicates_documents_options_and_indexes_but_not_onto_an_existing_name() {
    if !local_enabled("duplicates_documents_options_and_indexes_but_not_onto_an_existing_name") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let src = dbs.ns("src");
    store
        .create_collection(
            &src,
            &CollectionOptions {
                validator: Some(doc! { "n": { "$exists": true } }),
                ..CollectionOptions::default()
            },
        )
        .await
        .unwrap();
    seed(&client, &src, 4).await;
    client
        .database(&dbs.db)
        .run_command(
            doc! { "createIndexes": "src", "indexes": [ { "key": { "n": 1 }, "name": "n_1" } ] },
        )
        .await
        .unwrap();
    store.duplicate_collection(&src, "copy").await.unwrap();
    let copy = dbs.ns("copy");
    assert_eq!(store.estimated_count(&copy).await.unwrap(), 4);
    let names: Vec<String> = store
        .list_index_specs(&copy)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(names.contains(&"n_1".to_string()), "{names:?}");
    let options = store.collection_options(&copy).await.unwrap().unwrap();
    assert!(options.validator.is_some());
    assert_eq!(
        store.duplicate_collection(&src, "copy").await,
        Err(CoreError::NamespaceExists)
    );
    assert_eq!(
        store.duplicate_collection(&dbs.ns("missing"), "x").await,
        Err(CoreError::NotFound)
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn reports_collection_and_database_statistics() {
    if !local_enabled("reports_collection_and_database_statistics") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("s");
    seed(&client, &ns, 25).await;
    let stats = store.collection_stats(&ns).await.unwrap();
    assert_eq!(stats.document_count, 25);
    assert!(stats.data_size > 0 && stats.storage_size > 0);
    assert!(stats.index_sizes.iter().any(|(name, _)| name == "_id_"));
    let db_stats = store.database_stats(&dbs.db).await.unwrap();
    assert_eq!(db_stats.collections, 1);
    assert_eq!(db_stats.document_count, 25);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_read_only_connection_refuses_every_admin_write_and_changes_nothing() {
    if !local_enabled("a_read_only_connection_refuses_every_admin_write_and_changes_nothing") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    seed(&client, &dbs.ns("keep"), 2).await;
    let locked = ReadOnlyStore::wrap(Arc::new(store), Arc::new(AtomicBool::new(true)));
    let ns = dbs.ns("keep");
    let refused = [
        locked.drop_collection(&ns).await,
        locked.drop_database(&dbs.db).await,
        locked.rename_collection(&ns, "other", true).await,
        locked.create_view(&dbs.ns("v"), "keep", &[]).await,
        locked.duplicate_collection(&ns, "copy").await,
    ];
    assert!(refused.iter().all(|r| *r == Err(CoreError::ReadOnly)));
    assert_eq!(locked.estimated_count(&ns).await.unwrap(), 2);
    assert_eq!(
        locked.collection_stats(&ns).await.unwrap().document_count,
        2
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_failed_duplicate_leaves_no_half_made_copy_so_a_retry_can_run() {
    if !local_enabled("a_failed_duplicate_leaves_no_half_made_copy_so_a_retry_can_run") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("src");
    seed(&client, &ns, 3).await;
    // Documents inserted before the validator existed: the copy's merge is checked against it.
    client
        .database(&dbs.db)
        .run_command(doc! {
            "collMod": "src",
            "validator": { "n": { "$gt": 100 } },
            "validationAction": "error",
            "validationLevel": "strict",
        })
        .await
        .unwrap();
    let failed = store.duplicate_collection(&ns, "copy").await;
    assert!(
        failed.is_err(),
        "the merge must be refused by the validator"
    );
    assert!(
        !collections(&store, &dbs.db).await.contains(&"copy".into()),
        "the half-made target must be dropped"
    );
    // Removing the cause lets the same duplicate run: it is not stuck on NamespaceExists.
    client
        .database(&dbs.db)
        .run_command(doc! { "collMod": "src", "validator": {}, "validationLevel": "off" })
        .await
        .unwrap();
    store.duplicate_collection(&ns, "copy").await.unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn collection_level_writes_are_refused_in_server_owned_databases() {
    if !local_enabled("collection_level_writes_are_refused_in_server_owned_databases") {
        return;
    }
    let (_dbs, _client, store) = local_mongo().await;
    for db in ["admin", "local", "config"] {
        let ns = Namespace::new(db, "anything");
        let refused = Err(CoreError::ReservedDatabase);
        assert_eq!(store.drop_collection(&ns).await, refused);
        assert_eq!(store.rename_collection(&ns, "x", false).await, refused);
        assert_eq!(store.create_view(&ns, "src", &[]).await, refused);
        assert_eq!(store.duplicate_collection(&ns, "x").await, refused);
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_legacy_named_collection_and_database_can_be_dropped_and_a_view_has_no_statistics() {
    if !local_enabled(
        "a_legacy_named_collection_and_database_can_be_dropped_and_a_view_has_no_statistics",
    ) {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    // `system.js` is legal and existing; creation rules would have locked it out of a drop.
    seed(&client, &dbs.ns("system.js"), 1).await;
    store.drop_collection(&dbs.ns("system.js")).await.unwrap();
    seed(&client, &dbs.ns("src"), 2).await;
    store.create_view(&dbs.ns("v"), "src", &[]).await.unwrap();
    assert!(matches!(
        store.collection_stats(&dbs.ns("v")).await,
        Err(CoreError::Unsupported(_))
    ));
    // A database that is already gone is done, like a collection.
    store.drop_database(&dbs.db).await.unwrap();
    assert_eq!(store.drop_database(&dbs.db).await, Ok(()));
}
