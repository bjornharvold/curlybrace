//! Tier 1 (local server): the compare store contract against a real MongoDB. Writes only to
//! throwaway `cbit_*` databases. Run with `scripts/integration-tests.sh`.
//!
//! These tests are the main defence against the client comparator disagreeing with the server's
//! sort order. If one fails, DO NOT weaken it: report the failing pair.

mod support;

use std::cmp::Ordering;

use bson::oid::ObjectId;
use bson::spec::BinarySubtype;
use bson::{Binary, Bson, DateTime, Decimal128, Document, Regex, Timestamp, doc};
use curlybrace_core::compare::key::{KeySpec, KeyTuple, compare_bson, compare_tuples, extract_key};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{CollectionKind, DocumentStore, IndexStatus, Namespace};
use futures::TryStreamExt;
use mongodb::options::Collation;
use std::str::FromStr;
use support::{local_enabled, local_mongo};

fn dec(s: &str) -> Bson {
    Bson::Decimal128(Decimal128::from_str(s).unwrap())
}

/// Keys of every type class, with boundary numbers. Each appears once; none compare equal to
/// another (numbers that are equal across numeric types are covered by
/// `equal_numbers_of_different_types_are_equal_to_the_server`).
fn fixture_keys() -> Vec<Bson> {
    vec![
        Bson::MinKey,
        Bson::Null,
        Bson::Double(f64::NEG_INFINITY),
        Bson::Int64(i64::MIN),
        Bson::Int32(-5),
        Bson::Double(-0.5),
        Bson::Int32(0),
        dec("0.25"),
        Bson::Int32(1),
        Bson::Int64((1 << 53) + 1),
        Bson::Double(9_007_199_254_740_994.0),
        Bson::Int64(i64::MAX),
        Bson::Double(f64::INFINITY),
        Bson::String("".into()),
        Bson::String("B".into()),
        Bson::String("a".into()),
        Bson::String("ab".into()),
        Bson::String("z".into()),
        Bson::String("é".into()),
        Bson::Document(doc! { "a": 1 }),
        Bson::Document(doc! { "a": 2 }),
        Bson::Document(doc! { "b": 1 }),
        Bson::Document(doc! { "a": 1, "b": 1 }),
        Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: vec![0xff],
        }),
        Bson::Binary(Binary {
            subtype: BinarySubtype::Generic,
            bytes: vec![0, 0],
        }),
        Bson::Binary(Binary {
            subtype: BinarySubtype::Uuid,
            bytes: vec![0, 0],
        }),
        Bson::ObjectId(ObjectId::from_bytes([0; 12])),
        Bson::ObjectId(ObjectId::from_bytes([1; 12])),
        Bson::Boolean(false),
        Bson::Boolean(true),
        Bson::DateTime(DateTime::from_millis(-1)),
        Bson::DateTime(DateTime::from_millis(1)),
        Bson::Timestamp(Timestamp {
            time: 0,
            increment: 1,
        }),
        Bson::Timestamp(Timestamp {
            time: 1,
            increment: 0,
        }),
        Bson::RegularExpression(Regex {
            pattern: "a".into(),
            options: "".into(),
        }),
        Bson::MaxKey,
    ]
}

async fn seed(client: &mongodb::Client, ns: &Namespace, keys: &[Bson]) {
    let docs: Vec<Document> = keys.iter().map(|k| doc! { "k": k.clone() }).collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn the_client_comparator_orders_every_key_type_like_the_server() {
    if !local_enabled("the_client_comparator_orders_every_key_type_like_the_server") {
        return;
    }
    let (dbs, client, _store) = local_mongo().await;
    let ns = dbs.ns("order");
    let keys = fixture_keys();
    seed(&client, &ns, &keys).await;
    let cursor = client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find(doc! {})
        .sort(doc! { "k": 1 })
        .collation(Collation::builder().locale("simple").build())
        .await
        .unwrap();
    let from_server: Vec<Document> = cursor.try_collect().await.unwrap();
    let server_order: Vec<Bson> = from_server
        .iter()
        .map(|d| d.get("k").unwrap().clone())
        .collect();
    // Equal-comparing keys may come back in either order, so compare the SEQUENCE of classes of
    // equality: every adjacent pair must be non-decreasing under OUR comparator.
    for pair in server_order.windows(2) {
        assert_ne!(
            compare_bson(&pair[0], &pair[1]),
            Ordering::Greater,
            "the server put {:?} before {:?} but the client comparator says the opposite",
            pair[0],
            pair[1]
        );
    }
    assert_eq!(server_order.len(), keys.len());
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn equal_numbers_of_different_types_are_equal_to_the_server() {
    if !local_enabled("equal_numbers_of_different_types_are_equal_to_the_server") {
        return;
    }
    let (dbs, client, _store) = local_mongo().await;
    let ns = dbs.ns("equal");
    let group = [
        Bson::Int32(7),
        Bson::Int64(7),
        Bson::Double(7.0),
        dec("7.00"),
    ];
    seed(&client, &ns, &group).await;
    for probe in &group {
        let n = client
            .database(&ns.db)
            .collection::<Document>(&ns.collection)
            .count_documents(doc! { "k": probe.clone() })
            .await
            .unwrap();
        assert_eq!(n, group.len() as u64, "{probe:?} should match all four");
    }
    for a in &group {
        for b in &group {
            assert_eq!(compare_bson(a, b), Ordering::Equal, "{a:?} vs {b:?}");
        }
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn scan_sorted_pages_through_mixed_type_keys_completely_and_in_order() {
    if !local_enabled("scan_sorted_pages_through_mixed_type_keys_completely_and_in_order") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("mixed");
    let keys = vec![
        Bson::Int32(3),
        Bson::Int32(1),
        Bson::String("b".into()),
        Bson::String("a".into()),
        Bson::ObjectId(ObjectId::from_bytes([5; 12])),
        Bson::Boolean(true),
        Bson::Double(2.5),
        Bson::DateTime(DateTime::from_millis(10)),
        Bson::Null,
    ];
    seed(&client, &ns, &keys).await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    let mut seen: Vec<Bson> = Vec::new();
    let mut from: Option<KeyTuple> = None;
    loop {
        let batch = store
            .scan_sorted(&ns, &doc! {}, &spec, from.as_ref(), 2)
            .await
            .unwrap();
        if batch.is_empty() {
            break;
        }
        // The inclusive start returns the previous last row again: drop it.
        let fresh: Vec<&Document> = batch.iter().skip(usize::from(from.is_some())).collect();
        for d in &fresh {
            seen.push(d.get("k").unwrap().clone());
        }
        let last = extract_key(batch.last().unwrap(), &spec).unwrap();
        if batch.len() < 2 {
            break;
        }
        from = Some(last);
    }
    assert_eq!(
        seen.len(),
        keys.len(),
        "no document may be skipped at a type change: {seen:?}"
    );
    for pair in seen.windows(2) {
        assert_ne!(
            compare_bson(&pair[0], &pair[1]),
            Ordering::Greater,
            "{pair:?}"
        );
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn scan_sorted_applies_the_user_filter_and_compound_keys() {
    if !local_enabled("scan_sorted_applies_the_user_filter_and_compound_keys") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("compound");
    let docs: Vec<Document> = (0..20)
        .map(|n| doc! { "tenant": if n % 2 == 0 { "a" } else { "b" }, "n": n })
        .collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
    let spec = KeySpec::new(vec!["tenant".into(), "n".into()]).unwrap();
    let all = store
        .scan_sorted(&ns, &doc! { "tenant": "a" }, &spec, None, 100)
        .await
        .unwrap();
    assert_eq!(all.len(), 10);
    let from = extract_key(&all[3], &spec).unwrap();
    let tail = store
        .scan_sorted(&ns, &doc! { "tenant": "a" }, &spec, Some(&from), 100)
        .await
        .unwrap();
    assert_eq!(tail.len(), 7, "from the 4th document, inclusive");
    assert_eq!(tail[0], all[3]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn index_status_follows_the_real_indexes_and_collations() {
    if !local_enabled("index_status_follows_the_real_indexes_and_collations") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let db = client.database(&dbs.db);
    let spec = KeySpec::new(vec!["email".into()]).unwrap();
    // _id is always indexed.
    let ns_id = dbs.ns("with_id");
    db.collection::<Document>("with_id")
        .insert_one(doc! { "email": "a" })
        .await
        .unwrap();
    assert_eq!(
        store
            .key_index_status(&ns_id, &KeySpec::id())
            .await
            .unwrap(),
        IndexStatus::Indexed
    );
    assert_eq!(
        store.key_index_status(&ns_id, &spec).await.unwrap(),
        IndexStatus::NotIndexed
    );
    let coll = db.collection::<Document>("with_id");
    coll.create_index(
        mongodb::IndexModel::builder()
            .keys(doc! { "email": 1 })
            .build(),
    )
    .await
    .unwrap();
    assert_eq!(
        store.key_index_status(&ns_id, &spec).await.unwrap(),
        IndexStatus::Indexed
    );
    // A case-insensitive index cannot serve a simple-collation sort.
    let ns_ci = dbs.ns("ci");
    let ci = db.collection::<Document>("ci");
    ci.insert_one(doc! { "email": "a" }).await.unwrap();
    ci.create_index(
        mongodb::IndexModel::builder()
            .keys(doc! { "email": 1 })
            .options(
                mongodb::options::IndexOptions::builder()
                    .collation(
                        Collation::builder()
                            .locale("en")
                            .strength(mongodb::options::CollationStrength::Secondary)
                            .build(),
                    )
                    .build(),
            )
            .build(),
    )
    .await
    .unwrap();
    assert_eq!(
        store.key_index_status(&ns_ci, &spec).await.unwrap(),
        IndexStatus::NotIndexed
    );
    // A missing collection has nothing to sort.
    assert_eq!(
        store
            .key_index_status(&dbs.ns("absent"), &spec)
            .await
            .unwrap(),
        IndexStatus::Indexed
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn counts_kinds_and_missing_collections_behave() {
    if !local_enabled("counts_kinds_and_missing_collections_behave") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let db = client.database(&dbs.db);
    db.collection::<Document>("plain")
        .insert_many((0..5).map(|n| doc! { "n": n }))
        .await
        .unwrap();
    db.create_collection("a_view")
        .view_on("plain")
        .pipeline(vec![doc! { "$match": {} }])
        .await
        .unwrap();
    assert_eq!(store.estimated_count(&dbs.ns("plain")).await.unwrap(), 5);
    assert_eq!(store.estimated_count(&dbs.ns("absent")).await.unwrap(), 0);
    assert!(
        store
            .scan_sorted(&dbs.ns("absent"), &doc! {}, &KeySpec::id(), None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let kinds = store.list_collection_kinds(&dbs.db).await.unwrap();
    assert!(
        kinds.contains(&("plain".to_string(), CollectionKind::Collection)),
        "{kinds:?}"
    );
    assert!(
        kinds.contains(&("a_view".to_string(), CollectionKind::View)),
        "{kinds:?}"
    );
}

/// For every document as the start key, the scan from it must return exactly the documents whose
/// key is at or after it (by the client comparator), complete and in order. With a limit of 1 the
/// first row must carry the start key itself.
async fn assert_every_start_key_resumes(
    store: &curlybrace_core::mongo_store::MongoStore,
    ns: &Namespace,
    spec: &KeySpec,
    docs: &[Document],
) {
    for start in docs {
        let from = extract_key(start, spec).unwrap();
        let mut expected: Vec<String> = docs
            .iter()
            .filter(|d| compare_tuples(&extract_key(d, spec).unwrap(), &from) != Ordering::Less)
            .map(|d| d.get_str("id").unwrap().to_string())
            .collect();
        expected.sort();
        let got = store
            .scan_sorted(ns, &doc! {}, spec, Some(&from), 1000)
            .await
            .unwrap();
        let mut ids: Vec<String> = got
            .iter()
            .map(|d| d.get_str("id").unwrap().to_string())
            .collect();
        ids.sort();
        assert_eq!(ids, expected, "resuming from {from:?}");
        for pair in got.windows(2) {
            let (a, b) = (
                extract_key(&pair[0], spec).unwrap(),
                extract_key(&pair[1], spec).unwrap(),
            );
            assert_ne!(
                compare_tuples(&a, &b),
                Ordering::Greater,
                "{a:?} then {b:?}"
            );
        }
        for limit in [1, 2] {
            let head = store
                .scan_sorted(ns, &doc! {}, spec, Some(&from), limit)
                .await
                .unwrap();
            assert_eq!(
                head.len(),
                usize::try_from(limit).unwrap().min(expected.len())
            );
            assert_eq!(
                compare_tuples(&extract_key(&head[0], spec).unwrap(), &from),
                Ordering::Equal,
                "the first row at limit {limit} is the start key {from:?}"
            );
        }
    }
}

async fn insert_labelled(client: &mongodb::Client, ns: &Namespace, docs: &[Document]) {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs.to_vec())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn scan_sorted_resumes_from_nan_and_infinity_keys() {
    if !local_enabled("scan_sorted_resumes_from_nan_and_infinity_keys") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("nan");
    let keys = [
        Bson::Double(f64::NAN),
        dec("NaN"),
        Bson::Double(f64::NEG_INFINITY),
        Bson::Int32(-5),
        Bson::Int32(0),
        Bson::Double(2.5),
        dec("7"),
        Bson::Double(f64::INFINITY),
        Bson::String("s".into()),
    ];
    let docs: Vec<Document> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| doc! { "id": format!("d{i}"), "k": k.clone() })
        .collect();
    insert_labelled(&client, &ns, &docs).await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    assert_every_start_key_resumes(&store, &ns, &spec, &docs).await;
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn scan_sorted_resumes_compound_keys_with_nan_and_document_prefixes() {
    if !local_enabled("scan_sorted_resumes_compound_keys_with_nan_and_document_prefixes") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let ns = dbs.ns("compound_nan");
    let prefixes = [
        Bson::Double(f64::NAN),
        dec("NaN"),
        Bson::Int32(1),
        Bson::Double(f64::NEG_INFINITY),
    ];
    let mut docs = Vec::new();
    for (i, a) in prefixes.iter().enumerate() {
        for b in [1, 2] {
            docs.push(doc! { "id": format!("n{i}_{b}"), "a": a.clone(), "b": b });
        }
    }
    insert_labelled(&client, &ns, &docs).await;
    let spec = KeySpec::new(vec!["a".into(), "b".into()]).unwrap();
    assert_every_start_key_resumes(&store, &ns, &spec, &docs).await;

    // A document-valued prefix whose field name starts with `$`: skipped when the server will
    // not store it.
    let ns = dbs.ns("compound_dollar");
    let dollar: Vec<Document> = [1, 2]
        .iter()
        .flat_map(|b| {
            [doc! { "$x": 1 }, doc! { "y": 1 }]
                .into_iter()
                .enumerate()
                .map(move |(i, a)| doc! { "id": format!("o{i}_{b}"), "a": a, "b": *b })
        })
        .collect();
    let stored = client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(dollar.clone())
        .await;
    if stored.is_ok() {
        assert_every_start_key_resumes(&store, &ns, &spec, &dollar).await;
    } else {
        eprintln!("server refuses $-prefixed field names on insert; sub-case skipped");
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn scan_sorted_refuses_documents_without_a_usable_key() {
    if !local_enabled("scan_sorted_refuses_documents_without_a_usable_key") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let spec = KeySpec::new(vec!["k".into()]).unwrap();
    let missing = dbs.ns("missing_key");
    insert_labelled(&client, &missing, &[doc! { "k": 1 }, doc! { "other": 2 }]).await;
    assert_eq!(
        store.scan_sorted(&missing, &doc! {}, &spec, None, 10).await,
        Err(CoreError::CompareKeyMissing)
    );
    let array = dbs.ns("array_key");
    insert_labelled(&client, &array, &[doc! { "k": 1 }, doc! { "k": [1, 2] }]).await;
    assert_eq!(
        store.scan_sorted(&array, &doc! {}, &spec, None, 10).await,
        Err(CoreError::CompareKeyInvalid)
    );
}
