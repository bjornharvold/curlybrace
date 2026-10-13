//! Tier 1 (local server): `MongoStore::bulk_write` against a real MongoDB. Writes only to
//! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.

mod support;

use bson::oid::ObjectId;
use bson::{Bson, Document, doc};
use curlybrace_core::store::{DocumentStore, Namespace, WriteFailure, WriteOp};
use futures::TryStreamExt;
use support::{local_enabled, local_mongo};

async fn all_docs(client: &mongodb::Client, ns: &Namespace) -> Vec<Document> {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find(doc! {})
        .sort(doc! { "_id": 1 })
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_round_trips_all_three_kinds_on_a_real_server() {
    if !local_enabled("bulk_write_round_trips_all_three_kinds_on_a_real_server") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("kinds");
    let seed: Vec<Document> = (1..=3).map(|n| doc! { "_id": n, "v": "old" }).collect();
    support::insert_labelled(&client, &ns, &seed).await;
    let ops = vec![
        WriteOp::Insert {
            doc: doc! { "_id": 10, "v": "new" },
        },
        WriteOp::Replace {
            id: Bson::Int32(1),
            doc: doc! { "_id": 1, "v": "replaced" },
        },
        WriteOp::Delete { id: Bson::Int32(2) },
        WriteOp::Insert {
            doc: doc! { "v": "no id" },
        },
    ];
    let outcomes = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(outcomes, vec![Ok(()); 4]);
    let docs = all_docs(&client, &ns).await;
    assert_eq!(docs.len(), 4, "{docs:?}");
    assert!(docs.contains(&doc! { "_id": 1, "v": "replaced" }));
    assert!(docs.contains(&doc! { "_id": 3, "v": "old" }));
    assert!(docs.contains(&doc! { "_id": 10, "v": "new" }));
    assert!(docs.iter().all(|d| d.get("_id") != Some(&Bson::Int32(2))));
    assert!(
        docs.iter()
            .any(|d| d.get_str("v") == Ok("no id") && d.contains_key("_id"))
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_maps_a_duplicate_id_to_duplicate_key_and_keeps_going() {
    if !local_enabled("bulk_write_maps_a_duplicate_id_to_duplicate_key_and_keeps_going") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("dups");
    let secret = "dup-id-SECRET-4711";
    support::insert_labelled(&client, &ns, &[doc! { "_id": secret }]).await;
    let ops = vec![
        WriteOp::Insert {
            doc: doc! { "_id": "fresh-a" },
        },
        WriteOp::Insert {
            doc: doc! { "_id": secret },
        },
        WriteOp::Insert {
            doc: doc! { "_id": "fresh-b" },
        },
    ];
    let outcomes = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(
        outcomes,
        vec![Ok(()), Err(WriteFailure::DuplicateKey), Ok(())]
    );
    assert!(!format!("{outcomes:?}").contains(secret));
    assert_eq!(
        all_docs(&client, &ns).await.len(),
        3,
        "the others still landed"
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_reports_missing_ids_for_replace_and_delete() {
    if !local_enabled("bulk_write_reports_missing_ids_for_replace_and_delete") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("missing");
    support::insert_labelled(&client, &ns, &[doc! { "_id": 1 }, doc! { "_id": 2 }]).await;
    let ops = vec![
        WriteOp::Replace {
            id: Bson::Int32(1),
            doc: doc! { "_id": 1, "x": 1 },
        },
        WriteOp::Replace {
            id: Bson::Int32(99),
            doc: doc! { "_id": 99, "x": 1 },
        },
        WriteOp::Delete { id: Bson::Int32(2) },
        WriteOp::Delete {
            id: Bson::Int32(98),
        },
    ];
    let outcomes = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(
        outcomes,
        vec![
            Ok(()),
            Err(WriteFailure::Missing),
            Ok(()),
            Err(WriteFailure::Missing)
        ]
    );
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "x": 1 }]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_handles_more_than_500_ops_in_one_call() {
    if !local_enabled("bulk_write_handles_more_than_500_ops_in_one_call") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("many");
    let inserts: Vec<WriteOp> = (0..1203)
        .map(|n| WriteOp::Insert {
            doc: doc! { "_id": n },
        })
        .collect();
    let outcomes = store.bulk_write(&ns, inserts).await.unwrap();
    assert_eq!(outcomes, vec![Ok(()); 1203]);
    assert_eq!(all_docs(&client, &ns).await.len(), 1203);
    let deletes: Vec<WriteOp> = (0..1203)
        .map(|n| WriteOp::Delete { id: Bson::Int32(n) })
        .collect();
    let outcomes = store.bulk_write(&ns, deletes).await.unwrap();
    assert_eq!(outcomes, vec![Ok(()); 1203]);
    assert!(all_docs(&client, &ns).await.is_empty());
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_preserves_mixed_id_types() {
    if !local_enabled("bulk_write_preserves_mixed_id_types") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("mixed_ids");
    let ids = [
        Bson::ObjectId(ObjectId::new()),
        Bson::String("s".into()),
        Bson::Int64(1 << 40),
        Bson::Document(doc! { "a": 1, "b": "x" }),
    ];
    let inserts: Vec<WriteOp> = ids
        .iter()
        .map(|id| WriteOp::Insert {
            doc: doc! { "_id": id.clone(), "v": 1 },
        })
        .collect();
    assert_eq!(
        store.bulk_write(&ns, inserts).await.unwrap(),
        vec![Ok(()); 4]
    );
    let stored: Vec<Bson> = all_docs(&client, &ns)
        .await
        .into_iter()
        .map(|d| d.get("_id").unwrap().clone())
        .collect();
    for id in &ids {
        assert!(stored.contains(id), "{id:?} not stored as is: {stored:?}");
    }
    let replaces: Vec<WriteOp> = ids
        .iter()
        .map(|id| WriteOp::Replace {
            id: id.clone(),
            doc: doc! { "_id": id.clone(), "v": 2 },
        })
        .collect();
    assert_eq!(
        store.bulk_write(&ns, replaces).await.unwrap(),
        vec![Ok(()); 4]
    );
    assert!(
        all_docs(&client, &ns)
            .await
            .iter()
            .all(|d| d.get_i32("v") == Ok(2))
    );
    let deletes: Vec<WriteOp> = ids
        .iter()
        .map(|id| WriteOp::Delete { id: id.clone() })
        .collect();
    assert_eq!(
        store.bulk_write(&ns, deletes).await.unwrap(),
        vec![Ok(()); 4]
    );
    assert!(all_docs(&client, &ns).await.is_empty());
}

/// A replacement comes from user data: a `$`-prefixed field name is refused by the server and
/// must surface as `Rejected(code)`, as must a write into a view. Neither may panic or leak text.
#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_into_a_view_is_rejected() {
    if !local_enabled("bulk_write_into_a_view_is_rejected") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let db = client.database(&dbs.db);
    db.collection::<Document>("base")
        .insert_one(doc! { "_id": 1 })
        .await
        .unwrap();
    db.create_collection("a_view")
        .view_on("base")
        .pipeline(vec![doc! { "$match": {} }])
        .await
        .unwrap();
    let outcomes = store
        .bulk_write(
            &dbs.ns("a_view"),
            vec![WriteOp::Insert {
                doc: doc! { "_id": 5 },
            }],
        )
        .await
        .unwrap();
    assert!(
        matches!(outcomes[0], Err(WriteFailure::Rejected(_))),
        "{outcomes:?}"
    );
    let ns = dbs.ns("base");
    let outcomes = store
        .bulk_write(
            &ns,
            vec![WriteOp::Replace {
                id: Bson::Int32(1),
                doc: doc! { "_id": 1, "$bad": 1 },
            }],
        )
        .await
        .unwrap();
    assert!(
        matches!(outcomes[0], Err(WriteFailure::Rejected(_))),
        "{outcomes:?}"
    );
    assert_eq!(all_docs(&client, &ns).await, vec![doc! { "_id": 1 }]);
}

/// Replacement text comes from user data. A document made only of `$set` must never run as an
/// operator update: the op's `_id` goes first, so the server sees a replacement with a
/// `$`-field and refuses it.
#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_never_reads_a_replacement_as_an_operator_update() {
    if !local_enabled("bulk_write_never_reads_a_replacement_as_an_operator_update") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("operator");
    support::insert_labelled(&client, &ns, &[doc! { "_id": 1, "keep": "me" }]).await;
    let ops = vec![WriteOp::Replace {
        id: Bson::Int32(1),
        doc: doc! { "$set": { "x": 1 } },
    }];
    let outcomes = store.bulk_write(&ns, ops).await.unwrap();
    assert!(
        matches!(outcomes[0], Err(WriteFailure::Rejected(_))),
        "{outcomes:?}"
    );
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "keep": "me" }]
    );
}

/// An `_id` that is a sub-document with `$` keys is a literal value, not a query operator.
#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_treats_operator_shaped_ids_as_literals() {
    if !local_enabled("bulk_write_treats_operator_shaped_ids_as_literals") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("literal_ids");
    let seed = [doc! { "_id": "a", "v": 1 }, doc! { "_id": "b", "v": 1 }];
    support::insert_labelled(&client, &ns, &seed).await;
    let evil = Bson::Document(doc! { "$gt": "" });
    let ops = vec![
        WriteOp::Delete { id: evil.clone() },
        WriteOp::Replace {
            id: evil.clone(),
            doc: doc! { "_id": evil.clone(), "v": 2 },
        },
    ];
    let outcomes = store.bulk_write(&ns, ops).await.unwrap();
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, Err(WriteFailure::Missing | WriteFailure::Rejected(_)))),
        "{outcomes:?}"
    );
    assert_eq!(all_docs(&client, &ns).await, seed.to_vec());
}

/// The Missing probe compares ids as the server does: stored `1.0` is found by `Int32(1)`.
#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn bulk_write_probe_matches_numeric_and_sub_document_ids() {
    if !local_enabled("bulk_write_probe_matches_numeric_and_sub_document_ids") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("probe");
    let sub = Bson::Document(doc! { "k": 1 });
    let seed = [doc! { "_id": 1.0 }, doc! { "_id": sub.clone() }];
    support::insert_labelled(&client, &ns, &seed).await;
    let absent_sub = Bson::Document(doc! { "k": 2 });
    let replaces = vec![
        WriteOp::Replace {
            id: Bson::Int32(1),
            doc: doc! { "v": 1 },
        },
        WriteOp::Replace {
            id: sub.clone(),
            doc: doc! { "v": 1 },
        },
        WriteOp::Replace {
            id: Bson::Int32(7),
            doc: doc! { "v": 1 },
        },
        WriteOp::Replace {
            id: absent_sub.clone(),
            doc: doc! { "v": 1 },
        },
    ];
    let outcomes = store.bulk_write(&ns, replaces).await.unwrap();
    assert_eq!(
        outcomes,
        vec![
            Ok(()),
            Ok(()),
            Err(WriteFailure::Missing),
            Err(WriteFailure::Missing)
        ]
    );
    let deletes = vec![
        WriteOp::Delete { id: Bson::Int32(1) },
        WriteOp::Delete { id: sub },
        WriteOp::Delete { id: Bson::Int32(7) },
        WriteOp::Delete { id: absent_sub },
    ];
    let outcomes = store.bulk_write(&ns, deletes).await.unwrap();
    assert_eq!(
        outcomes,
        vec![
            Ok(()),
            Ok(()),
            Err(WriteFailure::Missing),
            Err(WriteFailure::Missing)
        ]
    );
}
