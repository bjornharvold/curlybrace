//! Tier 1 (local server): `duplicate_keys` against a real MongoDB, compared with the in-memory
//! store's semantics. Writes only to throwaway `cbit_*` databases. Run with
//! `scripts/integration-tests.sh`. If one fails, DO NOT weaken it: report the disagreement.

mod support;

use std::cmp::Ordering;

use bson::{Bson, Document, doc};
use curlybrace_core::compare::key::{KeySpec, KeyTuple, compare_tuples};
use curlybrace_core::store::{DocumentStore, DuplicateGroup};
use support::{insert_labelled, local_enabled, local_mongo};

fn dup_docs() -> Vec<Document> {
    vec![
        doc! { "_id": 1, "k": 1_i32, "t": "x" },
        doc! { "_id": 2, "k": 1_i64, "t": "x" },
        doc! { "_id": 3, "k": 1.0_f64, "t": "y" },
        doc! { "_id": 4, "k": "a" },
        doc! { "_id": 5, "k": "a", "t": "x" },
        doc! { "_id": 6, "k": "b" },
        doc! { "_id": 7, "t": "x" },
        doc! { "_id": 8, "t": "x" },
    ]
}

/// Group keys compared with the key comparator (the server keeps whichever of `1`, `1_i64`,
/// `1.0` it saw first as the representative), counts exactly.
fn assert_groups(actual: &[DuplicateGroup], expected: &[(Vec<Bson>, u64)]) {
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    for (group, (key, count)) in actual.iter().zip(expected) {
        assert_eq!(group.count, *count, "{group:?}");
        assert_eq!(
            compare_tuples(&group.key, &KeyTuple(key.clone())),
            Ordering::Equal,
            "{group:?} vs {key:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_matches_the_in_memory_semantics() {
    if !local_enabled("duplicate_keys_matches_the_in_memory_semantics") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("dups");
    insert_labelled(&client, &ns, &dup_docs()).await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    let groups = store
        .duplicate_keys(&ns, &doc! {}, &spec, 10)
        .await
        .unwrap();
    assert_groups(
        &groups,
        &[
            (vec![Bson::Int32(1)], 3),
            (vec![Bson::String("a".into())], 2),
        ],
    );
    let nans = dbs.ns("dup_nan");
    insert_labelled(
        &client,
        &nans,
        &[doc! { "k": f64::NAN }, doc! { "k": f64::NAN }],
    )
    .await;
    let groups = store
        .duplicate_keys(&nans, &doc! {}, &spec, 10)
        .await
        .unwrap();
    assert_eq!(
        groups.len(),
        1,
        "NaN groups with NaN on the server: {groups:?}"
    );
    assert_eq!(groups[0].count, 2);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_honours_filter_limit_and_compound_keys() {
    if !local_enabled("duplicate_keys_honours_filter_limit_and_compound_keys") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("dups_opts");
    insert_labelled(&client, &ns, &dup_docs()).await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    let only_x = store
        .duplicate_keys(&ns, &doc! { "t": "x" }, &spec, 10)
        .await
        .unwrap();
    assert_groups(&only_x, &[(vec![Bson::Int32(1)], 2)]);
    let limited = store.duplicate_keys(&ns, &doc! {}, &spec, 1).await.unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].count, 3);
    let compound = KeySpec::new(vec!["k".into(), "t".into()]).unwrap();
    let groups = store
        .duplicate_keys(&ns, &doc! {}, &compound, 10)
        .await
        .unwrap();
    assert_groups(
        &groups,
        &[(vec![Bson::Int32(1), Bson::String("x".into())], 2)],
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_with_a_dotted_key_and_array_crossing_paths() {
    if !local_enabled("duplicate_keys_with_a_dotted_key_and_array_crossing_paths") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("dups_dotted");
    insert_labelled(
        &client,
        &ns,
        &[
            doc! { "a": { "b": 1 } },
            doc! { "a": { "b": 1 } },
            // `a.b` crosses an array: the scanner (and `key_values`) treat it as missing.
            doc! { "a": [{ "b": 1 }] },
            doc! { "a": [{ "b": 1 }] },
            doc! { "a": [{ "b": 2 }, { "b": 3 }] },
            doc! { "a": [{ "b": 2 }, { "b": 3 }] },
        ],
    )
    .await;
    let spec = KeySpec::new(vec!["a.b".into()]).unwrap();
    let groups = store
        .duplicate_keys(&ns, &doc! {}, &spec, 10)
        .await
        .unwrap();
    assert_groups(&groups, &[(vec![Bson::Int32(1)], 2)]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_on_a_missing_collection_is_empty() {
    if !local_enabled("duplicate_keys_on_a_missing_collection_is_empty") {
        return;
    }
    let (dbs, _client, store) = local_mongo().await;
    let spec = KeySpec::id();
    let groups = store
        .duplicate_keys(&dbs.ns("never_created"), &doc! {}, &spec, 5)
        .await
        .unwrap();
    assert!(groups.is_empty());
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_refuses_dollar_prefixed_key_paths() {
    if !local_enabled("duplicate_keys_refuses_dollar_prefixed_key_paths") {
        return;
    }
    let (dbs, _client, store) = local_mongo().await;
    let spec = KeySpec::new(vec!["owner.$id".into()]).unwrap();
    let got = store
        .duplicate_keys(&dbs.ns("never_created"), &doc! {}, &spec, 5)
        .await;
    assert_eq!(
        got,
        Err(curlybrace_core::error::CoreError::CompareSetupInvalid)
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_cuts_ties_at_the_limit_in_key_order() {
    if !local_enabled("duplicate_keys_cuts_ties_at_the_limit_in_key_order") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("dups_many");
    // 30 groups of 2, inserted in descending key order so natural order cannot help.
    let docs: Vec<Document> = (0..60).rev().map(|n| doc! { "k": n / 2 }).collect();
    insert_labelled(&client, &ns, &docs).await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    let groups = store
        .duplicate_keys(&ns, &doc! {}, &spec, 21)
        .await
        .unwrap();
    let expected: Vec<(Vec<Bson>, u64)> = (0..21).map(|n| (vec![Bson::Int32(n)], 2)).collect();
    assert_groups(&groups, &expected);

    // Ties across type classes (null, number, string, document, array): the smallest keys win.
    let mixed = dbs.ns("dups_mixed");
    let values = [
        Bson::String("s".into()),
        Bson::Document(doc! { "z": 1 }),
        Bson::Array(vec![Bson::Int32(1)]),
        Bson::Int32(7),
        Bson::Null,
    ];
    let docs: Vec<Document> = values
        .iter()
        .flat_map(|v| [doc! { "k": v.clone() }, doc! { "k": v.clone() }])
        .collect();
    insert_labelled(&client, &mixed, &docs).await;
    let groups = store
        .duplicate_keys(&mixed, &doc! {}, &spec, 3)
        .await
        .unwrap();
    assert_groups(
        &groups,
        &[
            (vec![Bson::Null], 2),
            (vec![Bson::Int32(7)], 2),
            (vec![Bson::String("s".into())], 2),
        ],
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_with_a_zero_limit_is_empty() {
    if !local_enabled("duplicate_keys_with_a_zero_limit_is_empty") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("dups_zero");
    insert_labelled(&client, &ns, &dup_docs()).await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    let groups = store.duplicate_keys(&ns, &doc! {}, &spec, 0).await.unwrap();
    assert!(groups.is_empty());
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn duplicate_keys_groups_array_valued_keys_like_the_in_memory_store() {
    if !local_enabled("duplicate_keys_groups_array_valued_keys_like_the_in_memory_store") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("dups_arrays");
    let docs = vec![
        doc! { "k": [1_i32, 2_i32] },
        doc! { "k": [1.0_f64, 2_i32] },
        doc! { "k": [2_i32, 1_i32] },
    ];
    insert_labelled(&client, &ns, &docs).await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    let server = store
        .duplicate_keys(&ns, &doc! {}, &spec, 10)
        .await
        .unwrap();
    let memory = curlybrace_core::test_support::InMemoryStore::new()
        .with_docs(&ns, docs)
        .duplicate_keys(&ns, &doc! {}, &spec, 10)
        .await
        .unwrap();
    eprintln!("array keys: server {server:?} memory {memory:?}");
    assert_eq!(server.len(), memory.len());
    for (s, m) in server.iter().zip(&memory) {
        assert_eq!(s.count, m.count);
        assert_eq!(compare_tuples(&s.key, &m.key), Ordering::Equal);
    }
    assert_eq!(server.len(), 1);
    assert_eq!(
        server[0].count, 2,
        "[1,2] and [1.0,2] collide; [2,1] does not"
    );
}
