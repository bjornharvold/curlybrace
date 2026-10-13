//! Tier 1 (local server): `WriteOp::UpdateFields` through `MongoStore::bulk_write` against a
//! real MongoDB. Writes only to throwaway `cbit_*` databases. These tests additionally need an
//! explicit `CURLYBRACE_IT_MONGODB_URI` (a `cb-it-dev-*` container of your own, see
//! docs/superpowers/notes/testing.md); without it they skip with a message.

mod support;

use bson::{Bson, Document, doc};
use curlybrace_core::compare::diff::PathSegment::{self, Index as I};
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::store::{DocumentStore, Namespace, WriteFailure, WriteOp};
use futures::TryStreamExt;
use support::{LocalDbs, local_enabled, local_mongo};

fn f(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

/// The shared gate plus the rule that the server address must be chosen explicitly.
fn enabled(test: &str) -> bool {
    if !local_enabled(test) {
        return false;
    }
    let explicit = std::env::var(curlybrace_testkit::ENV_LOCAL_URI)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if !explicit {
        eprintln!(
            "SKIPPED {test}: set {} to your own cb-it-dev-* container",
            curlybrace_testkit::ENV_LOCAL_URI
        );
    }
    explicit
}

fn update(id: Bson, set: Vec<(Vec<PathSegment>, Bson)>, unset: Vec<Vec<PathSegment>>) -> WriteOp {
    WriteOp::UpdateFields { id, set, unset }
}

fn set1(path: Vec<PathSegment>, value: impl Into<Bson>) -> Vec<(Vec<PathSegment>, Bson)> {
    vec![(path, value.into())]
}

async fn setup(
    name: &str,
    seed: &[Document],
) -> (LocalDbs, mongodb::Client, MongoStore, Namespace) {
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns(name);
    support::insert_labelled(&client, &ns, seed).await;
    (dbs, client, store, ns)
}

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
async fn update_fields_sets_nested_fields_and_leaves_the_rest() {
    if !enabled("update_fields_sets_nested_fields_and_leaves_the_rest") {
        return;
    }
    let seed = [
        doc! { "_id": 1, "a": { "b": 1, "c": 2 }, "keep": "k" },
        doc! { "_id": 2, "x": 1 },
    ];
    let (_dbs, client, store, ns) = setup("sets", &seed).await;
    let set = vec![
        (vec![f("a"), f("b")], Bson::Int64(9)),
        (vec![f("a"), f("new")], Bson::String("v".into())),
        (vec![f("top")], Bson::Boolean(true)),
    ];
    let out = store
        .bulk_write(&ns, vec![update(Bson::Int32(1), set, vec![])])
        .await
        .unwrap();
    assert_eq!(out, vec![Ok(())]);
    let docs = all_docs(&client, &ns).await;
    assert_eq!(
        docs[0],
        doc! { "_id": 1, "a": { "b": 9_i64, "c": 2, "new": "v" }, "keep": "k", "top": true }
    );
    assert_eq!(docs[1], seed[1], "neighbour untouched");
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_unsets_a_field_and_keeps_siblings() {
    if !enabled("update_fields_unsets_a_field_and_keeps_siblings") {
        return;
    }
    let seed = [doc! { "_id": 1, "a": { "b": 1, "c": 2 }, "keep": "k", "gone": 1 }];
    let (_dbs, client, store, ns) = setup("unsets", &seed).await;
    let unset = vec![vec![f("a"), f("b")], vec![f("gone")]];
    let set = set1(vec![f("keep")], "k2");
    let out = store
        .bulk_write(&ns, vec![update(Bson::Int32(1), set, unset)])
        .await
        .unwrap();
    assert_eq!(out, vec![Ok(())]);
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "a": { "c": 2 }, "keep": "k2" }]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_sets_an_array_element_by_index_and_only_that_one() {
    if !enabled("update_fields_sets_an_array_element_by_index_and_only_that_one") {
        return;
    }
    let seed = [doc! { "_id": 1, "arr": [10, 20, { "k": 1, "j": 2 }] }];
    let (_dbs, client, store, ns) = setup("arr", &seed).await;
    let set = vec![
        (vec![f("arr"), I(0)], Bson::Int32(11)),
        (vec![f("arr"), I(2), f("k")], Bson::Int32(5)),
    ];
    let out = store
        .bulk_write(&ns, vec![update(Bson::Int32(1), set, vec![])])
        .await
        .unwrap();
    assert_eq!(out, vec![Ok(())]);
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "arr": [11, 20, { "k": 5, "j": 2 }] }]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_reports_missing_for_an_absent_id() {
    if !enabled("update_fields_reports_missing_for_an_absent_id") {
        return;
    }
    let seed = [doc! { "_id": 1, "x": 1 }];
    let (_dbs, client, store, ns) = setup("absent", &seed).await;
    let ops = vec![
        update(Bson::Int32(1), set1(vec![f("x")], 2), vec![]),
        update(Bson::Int32(99), set1(vec![f("x")], 2), vec![]),
        update(Bson::Int32(98), vec![], vec![vec![f("x")]]),
    ];
    let out = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(
        out,
        vec![
            Ok(()),
            Err(WriteFailure::Missing),
            Err(WriteFailure::Missing)
        ]
    );
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "x": 2 }]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_refuses_dotted_dollar_and_id_paths_without_touching_neighbours() {
    if !enabled("update_fields_refuses_dotted_dollar_and_id_paths_without_touching_neighbours") {
        return;
    }
    let seed = [doc! { "_id": 1, "a": { "b": 1 }, "x": 1 }];
    let (_dbs, client, store, ns) = setup("refuse", &seed).await;
    let bad_paths: Vec<Vec<PathSegment>> = vec![
        vec![f("a.b")],
        vec![f("$set")],
        vec![f("a"), f("$x")],
        vec![f("")],
        vec![f("_id")],
        vec![f("a"), PathSegment::AnyElement],
        vec![],
    ];
    for bad in bad_paths {
        let ops = vec![
            update(Bson::Int32(1), set1(bad.clone(), 5), vec![]),
            update(Bson::Int32(1), set1(vec![f("x")], 2), vec![]),
        ];
        let out = store.bulk_write(&ns, ops).await.unwrap();
        assert!(matches!(out[0], Err(WriteFailure::Rejected(_))), "{bad:?}");
        assert_eq!(out[1], Ok(()), "{bad:?}");
        let docs = all_docs(&client, &ns).await;
        assert_eq!(
            docs,
            vec![doc! { "_id": 1, "a": { "b": 1 }, "x": 2 }],
            "{bad:?}"
        );
        let reset = update(Bson::Int32(1), set1(vec![f("x")], 1), vec![]);
        store.bulk_write(&ns, vec![reset]).await.unwrap();
    }
    let seed_arr = doc! { "_id": 2, "arr": [1, { "k": 1 }] };
    support::insert_labelled(&client, &ns, &[seed_arr.clone()]).await;
    for path in [vec![f("arr"), I(0)], vec![f("arr"), I(1), f("k")]] {
        let op = update(Bson::Int32(2), vec![], vec![path.clone()]);
        let out = store.bulk_write(&ns, vec![op]).await.unwrap();
        assert_eq!(out, vec![Err(WriteFailure::Rejected(2))], "{path:?}");
    }
    assert_eq!(
        all_docs(&client, &ns).await[1],
        seed_arr,
        "unset through an array"
    );
    let conflict = update(
        Bson::Int32(1),
        set1(vec![f("a")], 0),
        vec![vec![f("a"), f("b")]],
    );
    let out = store.bulk_write(&ns, vec![conflict]).await.unwrap();
    assert_eq!(out, vec![Err(WriteFailure::Rejected(40))]);
    assert_eq!(
        all_docs(&client, &ns).await[0].get_document("a"),
        Ok(&doc! { "b": 1 })
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_mixes_with_insert_replace_delete_in_one_call_and_keeps_index_alignment() {
    if !enabled(
        "update_fields_mixes_with_insert_replace_delete_in_one_call_and_keeps_index_alignment",
    ) {
        return;
    }
    let seed: Vec<Document> = (1..=4)
        .map(|n| doc! { "_id": n, "v": "old", "w": 1 })
        .collect();
    let (_dbs, client, store, ns) = setup("mixed", &seed).await;
    let ops = vec![
        WriteOp::Delete { id: Bson::Int32(4) },
        update(Bson::Int32(1), set1(vec![f("v")], "fields"), vec![]),
        WriteOp::Insert {
            doc: doc! { "_id": 10, "v": "new" },
        },
        update(Bson::Int32(77), set1(vec![f("v")], "x"), vec![]),
        WriteOp::Replace {
            id: Bson::Int32(2),
            doc: doc! { "_id": 2, "v": "replaced" },
        },
        update(Bson::Int32(3), vec![], vec![vec![f("w")]]),
        update(Bson::Int32(3), set1(vec![f("_id")], 1), vec![]),
        WriteOp::Delete {
            id: Bson::Int32(66),
        },
    ];
    let out = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(out.len(), 8);
    assert_eq!(out[0], Ok(()));
    assert_eq!(out[1], Ok(()));
    assert_eq!(out[2], Ok(()));
    assert_eq!(out[3], Err(WriteFailure::Missing));
    assert_eq!(out[4], Ok(()));
    assert_eq!(out[5], Ok(()));
    assert_eq!(out[6], Err(WriteFailure::Rejected(66)));
    assert_eq!(out[7], Err(WriteFailure::Missing));
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![
            doc! { "_id": 1, "v": "fields", "w": 1 },
            doc! { "_id": 2, "v": "replaced" },
            doc! { "_id": 3, "v": "old" },
            doc! { "_id": 10, "v": "new" },
        ]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_with_large_values_splits_by_bytes() {
    if !enabled("update_fields_with_large_values_splits_by_bytes") {
        return;
    }
    let seed: Vec<Document> = (0..20).map(|n| doc! { "_id": n, "big": "old" }).collect();
    let (_dbs, client, store, ns) = setup("large", &seed).await;
    let big = "z".repeat(1_000_000);
    let ops: Vec<WriteOp> = (0..20)
        .map(|n| update(Bson::Int32(n), set1(vec![f("big")], big.clone()), vec![]))
        .collect();
    let out = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(out, vec![Ok(()); 20]);
    let docs = all_docs(&client, &ns).await;
    assert_eq!(docs.len(), 20);
    assert!(
        docs.iter()
            .all(|d| d.get_str("big").map(str::len) == Ok(1_000_000))
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_matches_a_sub_document_id_literally() {
    if !enabled("update_fields_matches_a_sub_document_id_literally") {
        return;
    }
    // The server forbids `$` keys inside a stored `_id`, so the operator-shaped id is an id
    // that matches nothing; with a bare `{ _id: {"$gt": ""} }` it would match every string id.
    let seed = [
        doc! { "_id": "other", "v": 1 },
        doc! { "_id": "zzz", "v": 1 },
        doc! { "_id": { "a": 1 }, "v": 1 },
    ];
    let (_dbs, client, store, ns) = setup("subid", &seed).await;
    let ops = vec![
        update(
            Bson::Document(doc! { "$gt": "" }),
            set1(vec![f("v")], 2),
            vec![],
        ),
        update(
            Bson::Document(doc! { "a": 1 }),
            set1(vec![f("v")], 3),
            vec![],
        ),
    ];
    let out = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(out, vec![Err(WriteFailure::Missing), Ok(())]);
    let docs = all_docs(&client, &ns).await;
    let mut values: Vec<(Bson, i32)> = docs
        .iter()
        .map(|d| (d.get("_id").unwrap().clone(), d.get_i32("v").unwrap()))
        .collect();
    values.sort_by_key(|(_, v)| *v);
    assert_eq!(values[2], (Bson::Document(doc! { "a": 1 }), 3), "{docs:?}");
    assert_eq!((values[0].1, values[1].1), (1, 1), "string ids untouched");
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_does_not_create_a_field_named_by_an_index_on_a_document() {
    if !enabled("update_fields_does_not_create_a_field_named_by_an_index_on_a_document") {
        return;
    }
    let seed = [doc! { "_id": 1, "a": { "b": 1 }, "arr": [1, 2] }];
    let (_dbs, client, store, ns) = setup("noindex", &seed).await;
    let ops = vec![
        update(Bson::Int32(1), set1(vec![f("a"), I(0)], 5), vec![]),
        update(Bson::Int32(1), set1(vec![f("arr"), f("0")], 5), vec![]),
    ];
    let out = store.bulk_write(&ns, ops).await.unwrap();
    assert!(
        out.iter()
            .all(|o| matches!(o, Err(WriteFailure::Missing | WriteFailure::Rejected(_)))),
        "{out:?}"
    );
    assert_eq!(all_docs(&client, &ns).await, seed.to_vec());
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_numeric_field_name_does_not_match_an_array_of_objects() {
    if !enabled("update_fields_numeric_field_name_does_not_match_an_array_of_objects") {
        return;
    }
    // Two documents: one op per id (the engine refuses duplicates; the probe cannot tell them apart).
    let seed = [
        doc! { "_id": 1, "arr": [{ "x": 1 }] },
        doc! { "_id": 2, "obj": { "0": 1 } },
    ];
    let (_dbs, client, store, ns) = setup("numfield", &seed).await;
    let ops = vec![
        update(Bson::Int32(1), set1(vec![f("arr"), f("0")], 5), vec![]),
        update(Bson::Int32(2), set1(vec![f("obj"), f("0")], 6), vec![]),
    ];
    let out = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(out, vec![Err(WriteFailure::Missing), Ok(())]);
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![
            doc! { "_id": 1, "arr": [{ "x": 1 }] },
            doc! { "_id": 2, "obj": { "0": 6 } }
        ]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_index_past_the_end_is_missing_and_does_not_pad() {
    if !enabled("update_fields_index_past_the_end_is_missing_and_does_not_pad") {
        return;
    }
    let seed = [doc! { "_id": 1, "arr": [1, 2] }];
    let (_dbs, client, store, ns) = setup("pad", &seed).await;
    let ops = vec![
        update(Bson::Int32(1), set1(vec![f("arr"), I(5)], 9), vec![]),
        update(Bson::Int32(1), set1(vec![f("arr"), I(2)], 9), vec![]),
    ];
    let out = store.bulk_write(&ns, ops).await.unwrap();
    assert_eq!(out, vec![Err(WriteFailure::Missing); 2]);
    assert_eq!(all_docs(&client, &ns).await, seed.to_vec());
    let ok = update(Bson::Int32(1), set1(vec![f("arr"), I(1)], 9), vec![]);
    assert_eq!(store.bulk_write(&ns, vec![ok]).await.unwrap(), vec![Ok(())]);
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "arr": [1, 9] }]
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_sets_through_nested_arrays_when_the_shape_is_right() {
    if !enabled("update_fields_sets_through_nested_arrays_when_the_shape_is_right") {
        return;
    }
    let seed = [doc! { "_id": 1, "a": [0, { "b": [10, 20], "c": 1 }] }];
    let (_dbs, client, store, ns) = setup("nested", &seed).await;
    let set = vec![
        (vec![f("a"), I(1), f("b"), I(0)], Bson::Int32(11)),
        (vec![f("a"), I(1), f("c")], Bson::Int32(2)),
    ];
    let out = store
        .bulk_write(&ns, vec![update(Bson::Int32(1), set, vec![])])
        .await
        .unwrap();
    assert_eq!(out, vec![Ok(())]);
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "a": [0, { "b": [11, 20], "c": 2 }] }]
    );
}

/// Pins what the server does with a VALUE that has `$`-prefixed keys.
#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn update_fields_value_with_dollar_keys_is_written_literally() {
    if !enabled("update_fields_value_with_dollar_keys_is_written_literally") {
        return;
    }
    let (_dbs, client, store, ns) = setup("dollarval", &[doc! { "_id": 1, "v": 0 }]).await;
    let value = Bson::Document(doc! { "$gt": 1, "$set": { "z": 1 } });
    let out = store
        .bulk_write(
            &ns,
            vec![update(
                Bson::Int32(1),
                set1(vec![f("v")], value.clone()),
                vec![],
            )],
        )
        .await
        .unwrap();
    assert_eq!(out, vec![Ok(())]);
    assert_eq!(
        all_docs(&client, &ns).await,
        vec![doc! { "_id": 1, "v": value }]
    );
}
