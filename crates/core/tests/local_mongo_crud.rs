//! Tier 1 (local server): CRUD, paging and document-service flows against an already-running
//! MongoDB (`CURLYBRACE_IT_MONGODB_URI`, default `mongodb://127.0.0.1:27017`). No docker. Writes
//! only to throwaway `cbit_*` databases.
//! Run with `scripts/integration-tests.sh`.

mod support;

use std::sync::Arc;

use bson::oid::ObjectId;
use bson::spec::{BinarySubtype, ElementType};
use bson::{Binary, Bson, Document, doc};
use curlybrace_core::crud::DocumentService;
use curlybrace_core::ejson::{doc_to_editor_json, editor_json_to_doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::query::{QueryText, next_page, parse_query};
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use curlybrace_core::test_support::mock_objects::exotic_doc;
use support::{local_enabled, local_mongo};

async fn seed_numbered(client: &mongodb::Client, ns: &Namespace, count: i32) {
    let docs: Vec<Document> = (0..count).map(|n| doc! { "_id": n, "n": n }).collect();
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .await
        .unwrap();
}

fn sorted_by_n() -> Query {
    Query {
        sort: Some(doc! { "n": 1 }),
        ..Query::default()
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn lists_databases_and_collections_with_awkward_names() {
    if !local_enabled("lists_databases_and_collections_with_awkward_names") {
        return;
    }
    let (dbs, _client, store) = local_mongo().await;
    let names = ["col.with.dots", "коллекция", "with space"];
    for name in names {
        let n = Namespace::new(&dbs.unicode_db, name);
        let id = store.insert(&n, doc! { "_id": 1_i32 }).await.unwrap();
        assert_eq!(id, Bson::Int32(1));
    }
    assert!(
        store
            .list_databases()
            .await
            .unwrap()
            .contains(&dbs.unicode_db)
    );
    let mut listed = store.list_collections(&dbs.unicode_db).await.unwrap();
    listed.sort();
    let mut want: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(listed, want);
    let back = store
        .find(
            &Namespace::new(&dbs.unicode_db, "col.with.dots"),
            &Query::default(),
        )
        .await
        .unwrap();
    assert_eq!(back.docs, vec![doc! { "_id": 1_i32 }]);
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn pages_of_fifty_over_120_100_and_101_documents() {
    if !local_enabled("pages_of_fifty_over_120_100_and_101_documents") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    // (document count, expected (len, has_more) per page)
    let cases: [(i32, &[(usize, bool)]); 3] = [
        (120, &[(50, true), (50, true), (20, false)]),
        (100, &[(50, true), (50, false)]),
        (101, &[(50, true), (50, true), (1, false)]),
    ];
    for (count, expected) in cases {
        let n = dbs.ns(&format!("paging_{count}"));
        seed_numbered(&client, &n, count).await;
        let mut q = sorted_by_n();
        let mut next_n = 0;
        for (page_no, (len, has_more)) in expected.iter().enumerate() {
            let page = store.find(&n, &q).await.unwrap();
            assert_eq!(page.docs.len(), *len, "{count} docs, page {page_no}");
            assert_eq!(page.has_more, *has_more, "{count} docs, page {page_no}");
            assert_eq!(page.skip, q.skip);
            for d in &page.docs {
                assert_eq!(d.get_i32("n"), Ok(next_n), "ordered across pages");
                next_n += 1;
            }
            q = next_page(&q);
        }
        assert_eq!(next_n, count);
        let beyond = store.find(&n, &q).await.unwrap();
        assert!(beyond.docs.is_empty() && !beyond.has_more);
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn sort_projection_skip_and_operator_filters() {
    if !local_enabled("sort_projection_skip_and_operator_filters") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let n = dbs.ns("queries");
    let docs: Vec<Document> = (0..10_i32)
        .map(|i| doc! { "_id": i, "age": i, "name": format!("p{i}") })
        .collect();
    client
        .database(&n.db)
        .collection::<Document>(&n.collection)
        .insert_many(docs)
        .await
        .unwrap();

    let q = Query {
        sort: Some(doc! { "age": -1 }),
        projection: Some(doc! { "age": 1 }),
        skip: 2,
        limit: 3,
        ..Query::default()
    };
    let page = store.find(&n, &q).await.unwrap();
    assert_eq!(
        page.docs,
        vec![
            doc! {"_id": 7, "age": 7},
            doc! {"_id": 6, "age": 6},
            doc! {"_id": 5, "age": 5}
        ]
    );
    assert!(page.has_more);

    let text = QueryText {
        filter: r#"{"age": {"$gt": 3}}"#.into(),
        sort: r#"{"age": 1}"#.into(),
        ..QueryText::default()
    };
    let page = store.find(&n, &parse_query(&text).unwrap()).await.unwrap();
    let ages: Vec<i32> = page
        .docs
        .iter()
        .map(|d| d.get_i32("age").unwrap())
        .collect();
    assert_eq!(ages, vec![4, 5, 6, 7, 8, 9]);
    assert!(!page.has_more);

    let text = QueryText {
        filter: r#"{"$or": [{"age": {"$lt": 1}}, {"name": {"$regex": "^p9$"}}]}"#.into(),
        sort: r#"{"age": 1}"#.into(),
        ..QueryText::default()
    };
    let page = store.find(&n, &parse_query(&text).unwrap()).await.unwrap();
    assert_eq!(page.docs.len(), 2);

    // A server-side rejection surfaces as a typed database error.
    let bad = Query {
        filter: doc! { "age": { "$bogus": 1 } },
        ..Query::default()
    };
    assert!(matches!(
        store.find(&n, &bad).await,
        Err(CoreError::Database(_))
    ));
}

fn uuid_id(seed: u8) -> Bson {
    Bson::Binary(Binary {
        subtype: BinarySubtype::Uuid,
        bytes: vec![seed; 16],
    })
}

fn id_kinds() -> Vec<(&'static str, Bson, Bson, Bson)> {
    vec![
        (
            "objectId",
            ObjectId::new().into(),
            ObjectId::new().into(),
            ObjectId::new().into(),
        ),
        ("string", "a".into(), "b".into(), "zz".into()),
        ("int32", Bson::Int32(1), Bson::Int32(2), Bson::Int32(99)),
        ("int64", Bson::Int64(1), Bson::Int64(2), Bson::Int64(99)),
        ("uuid", uuid_id(1), uuid_id(2), uuid_id(9)),
        (
            "compound",
            Bson::Document(doc! { "a": 1_i32, "b": "x" }),
            Bson::Document(doc! { "a": 2_i32, "b": "x" }),
            Bson::Document(doc! { "a": 9_i32, "b": "x" }),
        ),
    ]
}

fn seed_doc(id: &Bson) -> Document {
    doc! { "_id": id.clone(), "name": "alice" }
}

async fn all(store: &MongoStore, n: &Namespace) -> Vec<Document> {
    let q = Query {
        limit: 100,
        ..Query::default()
    };
    store.find(n, &q).await.unwrap().docs
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn replace_and_delete_hit_the_right_document_for_every_id_kind() {
    if !local_enabled("replace_and_delete_hit_the_right_document_for_every_id_kind") {
        return;
    }
    let (dbs, _client, store) = local_mongo().await;
    let mut counter = 0;
    for (kind, first, second, unknown) in id_kinds() {
        // Both seed orders, and both documents as target, prove that the id
        // filter (not insertion position) picks the document.
        for (order, target_is_first) in [(0, true), (0, false), (1, true), (1, false)] {
            counter += 1;
            let n = dbs.ns(&format!("ids_{counter}"));
            let (seed_a, seed_b) = if order == 0 {
                (&first, &second)
            } else {
                (&second, &first)
            };
            store.insert(&n, seed_doc(seed_a)).await.unwrap();
            store.insert(&n, seed_doc(seed_b)).await.unwrap();
            let (target, other) = if target_is_first {
                (&first, &second)
            } else {
                (&second, &first)
            };
            let label = format!("{kind} order={order} target_first={target_is_first}");

            let edited = doc! { "_id": target.clone(), "name": "edited", "extra": 7_i64 };
            store
                .replace(&n, target, edited.clone())
                .await
                .unwrap_or_else(|e| panic!("{label}: {e}"));
            let docs = all(&store, &n).await;
            assert_eq!(docs.len(), 2, "{label}");
            assert!(docs.contains(&edited), "{label}: target replaced");
            assert!(docs.contains(&seed_doc(other)), "{label}: other untouched");

            assert_eq!(
                store
                    .replace(&n, &unknown, doc! { "_id": unknown.clone() })
                    .await,
                Err(CoreError::NotFound),
                "{label}"
            );
            assert_eq!(
                store.delete(&n, &unknown).await,
                Err(CoreError::NotFound),
                "{label}"
            );

            store
                .delete(&n, target)
                .await
                .unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(all(&store, &n).await, vec![seed_doc(other)], "{label}");
            assert_eq!(
                store.delete(&n, target).await,
                Err(CoreError::NotFound),
                "{label}: second delete"
            );
        }
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn insert_generates_an_object_id_when_absent() {
    if !local_enabled("insert_generates_an_object_id_when_absent") {
        return;
    }
    let (dbs, _client, store) = local_mongo().await;
    let n = dbs.ns("autoid");
    let id = store.insert(&n, doc! { "x": 1_i32 }).await.unwrap();
    assert!(matches!(id, Bson::ObjectId(_)));
    let docs = all(&store, &n).await;
    assert_eq!(docs[0].get("_id"), Some(&id));
}

fn kinds(d: &Document) -> Vec<(String, ElementType)> {
    d.iter()
        .map(|(k, v)| (k.clone(), v.element_type()))
        .collect()
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn exotic_types_survive_insert_find_and_editor_round_trip() {
    if !local_enabled("exotic_types_survive_insert_find_and_editor_round_trip") {
        return;
    }
    let (dbs, client, store) = local_mongo().await;
    let n = dbs.ns("exotic");
    let store = Arc::new(store);
    let svc = DocumentService::new(store.clone());

    let mut original = exotic_doc();
    original.insert("dateMin", bson::DateTime::from_millis(i64::MIN));
    original.insert("dateMax", bson::DateTime::from_millis(i64::MAX));
    original.insert("small64", 5_i64);
    original.insert(
        "uuid",
        Binary {
            subtype: BinarySubtype::Uuid,
            bytes: vec![3; 16],
        },
    );
    let id = svc
        .insert_from_json(&n, &doc_to_editor_json(&original))
        .await
        .unwrap();
    assert_eq!(Some(&id), original.get("_id"));

    let q = Query {
        filter: doc! { "_id": id.clone() },
        ..Query::default()
    };
    let found = store.find(&n, &q).await.unwrap().docs.remove(0);
    // NaN != NaN under PartialEq, so compare the type-preserving text form.
    assert_eq!(doc_to_editor_json(&found), doc_to_editor_json(&original));
    assert_eq!(
        kinds(&found),
        kinds(&original),
        "server preserved every BSON type"
    );
    for (key, ty) in [
        ("int32", ElementType::Int32),
        ("small64", ElementType::Int64),
        ("decimal", ElementType::Decimal128),
        ("minKey", ElementType::MinKey),
        ("maxKey", ElementType::MaxKey),
        ("timestamp", ElementType::Timestamp),
        ("regex", ElementType::RegularExpression),
    ] {
        assert_eq!(found.get(key).unwrap().element_type(), ty, "{key}");
    }
    assert_eq!(
        found.get_datetime("dateMin").unwrap().timestamp_millis(),
        i64::MIN
    );
    assert_eq!(
        found.get_datetime("dateMax").unwrap().timestamp_millis(),
        i64::MAX
    );

    // Editor text from the stored document parses back to the same document.
    let reparsed = editor_json_to_doc(&doc_to_editor_json(&found)).unwrap();
    assert_eq!(doc_to_editor_json(&reparsed), doc_to_editor_json(&found));

    // Edit through the service: changes one field, keeps types of the rest.
    let mut edited = reparsed.clone();
    edited.insert("int32", 8_i32);
    svc.replace_from_json(&n, &id, &doc_to_editor_json(&edited))
        .await
        .unwrap();
    let after = store.find(&n, &q).await.unwrap().docs.remove(0);
    assert_eq!(after.get_i32("int32"), Ok(8));
    assert_eq!(kinds(&after), kinds(&original));

    // _id is immutable and a missing target is NotFound.
    let moved = doc! { "_id": 12345_i32 };
    assert!(matches!(
        svc.replace_from_json(&n, &id, &doc_to_editor_json(&moved))
            .await,
        Err(CoreError::InvalidDocument(_))
    ));
    let ghost = Bson::Int32(777);
    assert_eq!(
        svc.replace_from_json(&n, &ghost, &doc_to_editor_json(&doc! { "_id": 777_i32 }))
            .await,
        Err(CoreError::NotFound)
    );
    assert!(matches!(
        svc.insert_from_json(&n, "{not json").await,
        Err(CoreError::InvalidJson(_))
    ));

    // Clone gets a fresh _id and the same remaining fields; delete removes it.
    let copy_id = svc.clone_document(&n, &after).await.unwrap();
    assert_ne!(copy_id, id);
    let copy = store
        .find(
            &n,
            &Query {
                filter: doc! { "_id": copy_id.clone() },
                ..Query::default()
            },
        )
        .await
        .unwrap()
        .docs
        .remove(0);
    let mut expected = after.clone();
    expected.insert("_id", copy_id.clone());
    assert_eq!(doc_to_editor_json(&copy), doc_to_editor_json(&expected));
    svc.delete(&n, &copy_id).await.unwrap();
    svc.delete(&n, &id).await.unwrap();
    assert!(all(&store, &n).await.is_empty());
    assert_eq!(svc.delete(&n, &id).await, Err(CoreError::NotFound));
    drop(client);
}
