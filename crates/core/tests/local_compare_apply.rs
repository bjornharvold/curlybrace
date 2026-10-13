//! Tier 1 (local server): `MongoStore::bulk_write` against a real MongoDB. Writes only to
//! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.

mod support;

use bson::oid::ObjectId;
use bson::{Bson, Document, doc};
use curlybrace_core::compare::apply::{ApplyKind, IdPolicy};
use curlybrace_core::compare::protocol::{CompareCommand, CompareEvent};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{DocumentStore, Namespace, WriteFailure, WriteOp};
use futures::TryStreamExt;
use support::compare_flow::{
    SESSION, all_docs as sorted_docs, apply, compare, key, keys, op, open, pair, send,
    summary_tuple,
};
use support::{local_enabled, local_mongo, local_runtime, next_compare};

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

// ---- end-to-end apply flows through the runtime (Command::Compare) ----

fn ids(docs: &[Document]) -> Vec<Bson> {
    docs.iter().map(|d| d.get("_id").unwrap().clone()).collect()
}

/// Seeds `docs` into `ns`, creating the collection even when `docs` is empty.
async fn seed(client: &mongodb::Client, ns: &Namespace, docs: Vec<Document>) {
    let db = client.database(&ns.db);
    if docs.is_empty() {
        db.create_collection(&ns.collection).await.unwrap();
    } else {
        db.collection::<Document>(&ns.collection)
            .insert_many(docs)
            .await
            .unwrap();
    }
}

/// Insert/replace/delete ops for every row of a pair (source copied onto target).
async fn all_ops(
    rt: &mut curlybrace_core::runtime::RuntimeHandle,
    pair_id: u32,
) -> Vec<curlybrace_core::compare::apply::ApplyOp> {
    let mut ops = Vec::new();
    for (category, kind) in [
        (Category::OnlyInSource, ApplyKind::Insert),
        (Category::Different, ApplyKind::Replace),
        (Category::OnlyInTarget, ApplyKind::Delete),
    ] {
        for k in keys(rt, pair_id, category).await {
            ops.push(op(pair_id, k, kind));
        }
    }
    ops
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn compare_then_apply_then_compare_again_converges() {
    if !local_enabled("compare_then_apply_then_compare_again_converges") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let mut pairs = Vec::new();
    for (n, name) in ["one", "two"].into_iter().enumerate() {
        let (src, dst) = (
            dbs.ns(&format!("{name}_src")),
            dbs.ns(&format!("{name}_dst")),
        );
        let docs = |v: i32| {
            (1..=5)
                .map(|i| doc! { "_id": i, "v": v })
                .collect::<Vec<_>>()
        };
        seed(&client, &src, docs(1)).await;
        // 1,2 missing; 3 differs; 4,5 equal; 6,7 extra
        let mut target = vec![doc! { "_id": 3, "v": 99 }];
        target.extend(docs(1).into_iter().skip(3));
        target.extend([doc! { "_id": 6, "v": 1 }, doc! { "_id": 7, "v": 1 }]);
        seed(&client, &dst, target).await;
        pairs.push(pair(n as u32 + 1, src, dst, &["_id"]));
    }
    open(&mut rt, &config).await;
    let first = compare(&mut rt, 1, pairs.clone()).await;
    assert!(first.iter().all(|(_, s)| summary_tuple(s) == (2, 2, 2, 1)));
    let mut ops = all_ops(&mut rt, 1).await;
    ops.extend(all_ops(&mut rt, 2).await);
    assert_eq!(ops.len(), 10);
    let applied = apply(&mut rt, 2, ops, IdPolicy::Keep).await;
    assert_eq!(applied.reports.len(), 2);
    for (_, r) in &applied.reports {
        assert_eq!(
            (r.applied, r.failed, r.stale, r.not_run),
            (5, 0, 0, 0),
            "{r:?}"
        );
    }
    for name in ["one", "two"] {
        let got = sorted_docs(&client, &dbs.ns(&format!("{name}_dst"))).await;
        let want = sorted_docs(&client, &dbs.ns(&format!("{name}_src"))).await;
        assert_eq!(got, want);
    }
    for (_, s) in compare(&mut rt, 3, pairs).await {
        assert_eq!(summary_tuple(&s), (5, 0, 0, 0), "second run converges");
    }
    send(&rt, CompareCommand::Close { session: SESSION });
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn business_key_apply_keeps_destination_ids_and_converges() {
    if !local_enabled("business_key_apply_keeps_destination_ids_and_converges") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("bk_src"), dbs.ns("bk_dst"));
    let user = |e: &str, n: i32| doc! { "_id": ObjectId::new(), "email": e, "n": n };
    seed(
        &client,
        &src,
        vec![user("new@x", 1), user("chg@x", 2), user("same@x", 3)],
    )
    .await;
    let (chg, gone) = (user("chg@x", 20), user("gone@x", 4));
    seed(&client, &dst, vec![chg.clone(), gone, user("same@x", 3)]).await;
    let p = pair(1, src.clone(), dst.clone(), &["email"]);
    assert!(
        !p.rules.include_id,
        "business key leaves _id out of the compare"
    );
    open(&mut rt, &config).await;
    let first = compare(&mut rt, 1, vec![p.clone()]).await;
    assert_eq!(summary_tuple(&first[0].1), (1, 1, 1, 1));
    let ops = all_ops(&mut rt, 1).await;
    let applied = apply(&mut rt, 2, ops, IdPolicy::Keep).await;
    assert_eq!(
        applied.reports[0].1.applied, 3,
        "{:?}",
        applied.reports[0].1
    );
    let after = sorted_docs(&client, &dst).await;
    let chg_after = after
        .iter()
        .find(|d| d.get_str("email") == Ok("chg@x"))
        .unwrap();
    assert_eq!(
        chg_after.get("_id"),
        chg.get("_id"),
        "replace keeps the destination _id"
    );
    assert_eq!(chg_after.get_i32("n"), Ok(2));
    assert!(after.iter().all(|d| d.get_str("email") != Ok("gone@x")));
    assert!(after.iter().any(|d| d.get_str("email") == Ok("new@x")));
    let again = compare(&mut rt, 3, vec![p]).await;
    assert_eq!(summary_tuple(&again[0].1), (3, 0, 0, 0));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_stale_document_is_skipped_on_a_real_server() {
    if !local_enabled("a_stale_document_is_skipped_on_a_real_server") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("st_src"), dbs.ns("st_dst"));
    seed(
        &client,
        &src,
        vec![doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 1 }],
    )
    .await;
    seed(&client, &dst, vec![doc! { "_id": 2, "v": 2 }]).await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst.clone(), &["_id"])]).await;
    let ops = all_ops(&mut rt, 1).await;
    // A second client changes the target after the compare.
    client
        .database(&dst.db)
        .collection::<Document>(&dst.collection)
        .update_one(doc! { "_id": 2 }, doc! { "$set": { "v": 3 } })
        .await
        .unwrap();
    let applied = apply(&mut rt, 2, ops, IdPolicy::Keep).await;
    let r = &applied.reports[0].1;
    assert_eq!((r.applied, r.stale, r.failed), (1, 1, 0), "{r:?}");
    let after = sorted_docs(&client, &dst).await;
    assert_eq!(
        after,
        vec![doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 3 }]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn apply_with_generate_ids_assigns_new_ids() {
    if !local_enabled("apply_with_generate_ids_assigns_new_ids") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("gen_src"), dbs.ns("gen_dst"));
    let docs: Vec<Document> = (1..=3).map(|i| doc! { "_id": i, "a": i * 10 }).collect();
    seed(&client, &src, docs).await;
    seed(&client, &dst, vec![]).await;
    let p = pair(1, src, dst.clone(), &["_id"]);
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![p.clone()]).await;
    let ops = all_ops(&mut rt, 1).await;
    let applied = apply(&mut rt, 2, ops, IdPolicy::Generate).await;
    assert_eq!(applied.reports[0].1.applied, 3);
    let after = sorted_docs(&client, &dst).await;
    assert_eq!(after.len(), 3);
    assert!(ids(&after).iter().all(|id| matches!(id, Bson::ObjectId(_))));
    let mut values: Vec<i32> = after.iter().map(|d| d.get_i32("a").unwrap()).collect();
    values.sort();
    assert_eq!(values, vec![10, 20, 30]);
    // Documented non-convergence: the copies have new _ids, so an _id-keyed re-compare still
    // sees three source-only and three target-only rows.
    let again = compare(&mut rt, 3, vec![p]).await;
    assert_eq!(summary_tuple(&again[0].1), (0, 3, 3, 0));
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_unique_index_violation_is_a_per_document_failure() {
    if !local_enabled("a_unique_index_violation_is_a_per_document_failure") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("uq_src"), dbs.ns("uq_dst"));
    let secret = "taken-SECRET@example.com";
    let user = |id: i32, e: &str| doc! { "_id": id, "email": e };
    seed(
        &client,
        &src,
        vec![user(10, "a@x"), user(11, secret), user(12, "c@x")],
    )
    .await;
    seed(&client, &dst, vec![user(1, secret)]).await;
    client
        .database(&dst.db)
        .collection::<Document>(&dst.collection)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! { "email": 1 })
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst.clone(), &["_id"])]).await;
    let ops: Vec<_> = keys(&mut rt, 1, Category::OnlyInSource)
        .await
        .into_iter()
        .map(|k| op(1, k, ApplyKind::Insert))
        .collect();
    let applied = apply(&mut rt, 2, ops, IdPolicy::Keep).await;
    let r = &applied.reports[0].1;
    assert_eq!((r.applied, r.failed, r.stale), (2, 1, 0), "{r:?}");
    assert_eq!(r.failures[0].key, key(11));
    assert!(!format!("{r:?}").contains(secret));
    assert!(!r.failures[0].error.to_string().contains(secret));
    assert_eq!(sorted_docs(&client, &dst).await.len(), 3);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn apply_refuses_a_view_destination() {
    if !local_enabled("apply_refuses_a_view_destination") {
        return;
    }
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, base, view) = (dbs.ns("vw_src"), dbs.ns("vw_base"), dbs.ns("vw_view"));
    seed(&client, &src, vec![doc! { "_id": 1 }]).await;
    seed(&client, &base, vec![doc! { "_id": 2 }]).await;
    client
        .database(&view.db)
        .create_collection(&view.collection)
        .view_on(&base.collection)
        .pipeline(vec![doc! { "$match": {} }])
        .await
        .unwrap();
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, view, &["_id"])]).await;
    let k = keys(&mut rt, 1, Category::OnlyInSource).await.remove(0);
    send(
        &rt,
        CompareCommand::Apply {
            session: SESSION,
            run: 2,
            ops: vec![op(1, k, ApplyKind::Insert)],
            id_policy: IdPolicy::Keep,
        },
    );
    match next_compare(&mut rt).await {
        CompareEvent::Failed { context, error, .. } => {
            assert_eq!(context, "compare_apply");
            assert_eq!(error, CoreError::ApplyTargetNotWritable);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(sorted_docs(&client, &base).await, vec![doc! { "_id": 2 }]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn cancel_mid_apply_on_a_large_set() {
    if !local_enabled("cancel_mid_apply_on_a_large_set") {
        return;
    }
    const N: i32 = 5_000;
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("big_src"), dbs.ns("big_dst"));
    let docs: Vec<Document> = (0..N).map(|i| doc! { "_id": i, "v": i }).collect();
    seed(&client, &src, docs).await;
    seed(&client, &dst, vec![]).await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst.clone(), &["_id"])]).await;
    let ops = all_ops(&mut rt, 1).await;
    assert_eq!(ops.len(), N as usize);
    send(
        &rt,
        CompareCommand::Apply {
            session: SESSION,
            run: 2,
            ops,
            id_policy: IdPolicy::Keep,
        },
    );
    let mut report = None;
    let mut cancelled = false;
    loop {
        match next_compare(&mut rt).await {
            CompareEvent::ApplyProgress { .. } if !cancelled => {
                cancelled = true;
                send(
                    &rt,
                    CompareCommand::Cancel {
                        session: SESSION,
                        run: 2,
                    },
                );
            }
            CompareEvent::ApplyProgress { .. } => {}
            CompareEvent::ApplyPairDone { report: r, .. } => report = Some(r),
            CompareEvent::ApplyDone { .. } => break,
            other => panic!("{other:?}"),
        }
    }
    let r = report.expect("a report for the pair");
    assert!(cancelled && r.not_run > 0, "{r:?}");
    assert_eq!(r.applied + r.not_run, N as u64, "{r:?}");
    assert_eq!(r.failed + r.stale, 0);
    let count = sorted_docs(&client, &dst).await.len() as u64;
    assert_eq!(
        count, r.applied,
        "the target holds exactly the applied documents"
    );
}
