//! Manual integration test against a local mongod (`docker run -d -p 27017:27017 mongo:7`).
//! Run with: `cargo test -p mongostudio-core --test mongo_store_round_trip -- --ignored`.

use bson::{Bson, doc};
use mongostudio_core::client::connect;
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::error::CoreError;
use mongostudio_core::mongo_store::MongoStore;
use mongostudio_core::store::{DocumentStore, Namespace, Query};

const URI: &str = "mongodb://127.0.0.1:27017";

#[tokio::test]
#[ignore = "needs a mongod on 127.0.0.1:27017"]
async fn mongo_store_round_trip() {
    let cfg = ConnectionConfig::new("it", URI);
    let client = match connect(&cfg, None, None).await {
        Ok(client) => client,
        Err(e) => {
            eprintln!("skipping: mongod unreachable at {URI}: {e}");
            return;
        }
    };
    let cleanup = client.clone();
    let store = MongoStore::new(client);
    let ns = Namespace::new(
        "mongostudio_it",
        format!("c_{}", uuid::Uuid::new_v4().simple()),
    );

    for n in 0..5_i32 {
        store.insert(&ns, doc! { "_id": n, "n": n }).await.unwrap();
    }
    let q = Query {
        sort: Some(doc! { "n": -1 }),
        projection: Some(doc! { "n": 1 }),
        skip: 1,
        limit: 3,
        ..Query::default()
    };
    let page = store.find(&ns, &q).await.unwrap();
    assert_eq!(page.docs.len(), 3);
    assert!(page.has_more);
    assert_eq!(page.docs[0], doc! { "_id": 3, "n": 3 });

    store
        .replace(&ns, &Bson::Int32(0), doc! { "_id": 0, "n": 100 })
        .await
        .unwrap();
    assert_eq!(
        store
            .replace(&ns, &Bson::Int32(99), doc! { "_id": 99 })
            .await,
        Err(CoreError::NotFound)
    );
    store.delete(&ns, &Bson::Int32(0)).await.unwrap();
    assert_eq!(
        store.delete(&ns, &Bson::Int32(0)).await,
        Err(CoreError::NotFound)
    );

    assert!(store.list_databases().await.unwrap().contains(&ns.db));
    assert!(
        store
            .list_collections(&ns.db)
            .await
            .unwrap()
            .contains(&ns.collection)
    );
    cleanup.database(&ns.db).drop().await.unwrap();
}
