//! Tier 1 (local server, real `mongosh`): the exported mongosh script against a real MongoDB.
//! The script is run with the REAL shell inside a `cb-it-dev-*` container of our own (see
//! `support/mongosh.rs` and docs/superpowers/notes/testing.md). Writes only to throwaway
//! `cbit_*` databases. Needs `CURLYBRACE_IT=1`, an explicit `CURLYBRACE_IT_MONGODB_URI` and, for
//! these tests, `CURLYBRACE_IT_MONGOSH_CONTAINER`; without them each test skips with a message.

#![recursion_limit = "512"]

mod support;

use bson::oid::ObjectId;
use bson::spec::BinarySubtype;
use bson::{Binary, Bson, DateTime, Decimal128, Document, Regex, Timestamp, doc};
use curlybrace_core::compare::apply::IdPolicy;
use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::export::mongosh::INCOMPLETE_LINE;
use curlybrace_core::compare::protocol::ExportFormat;
use curlybrace_core::store::Namespace;
use support::compare_flow::{SESSION, all_docs, apply, compare, open, pair, send, summary_tuple};
use support::export_flow::{
    all_ops, enabled_with_mongosh, export_ok, field_bytes, fields_op, for_pair, read, request, seed,
};
use support::local_runtime;
use support::mongosh::{container_name_problem, run_script};

fn script_request(
    ops: Vec<curlybrace_core::compare::apply::ApplyOp>,
) -> curlybrace_core::compare::protocol::ExportRequest {
    request(ExportFormat::Mongosh, ops)
}

fn f(name: &str) -> PathSegment {
    PathSegment::Field(name.into())
}

async fn raw_docs(client: &mongodb::Client, ns: &Namespace) -> Vec<Document> {
    all_docs(client, ns).await
}

async fn count(client: &mongodb::Client, ns: &Namespace) -> u64 {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .count_documents(doc! {})
        .await
        .unwrap()
}

#[test]
fn the_mongosh_helper_refuses_any_container_that_is_not_ours() {
    for bad in [
        "local7979",
        "local265",
        "cb-it-",
        "cb-it-dev",
        "mongo",
        "x-cb-it-dev-3",
        "cb-it-dev-3 local7979",
        "cb-it-dev-3;rm",
        "cb-it-dev-$(id)",
        "",
    ] {
        assert!(container_name_problem(bad).is_some(), "{bad:?}");
    }
    for good in ["cb-it-dev-3", "cb-it-dev-slice_3"] {
        assert!(container_name_problem(good).is_none(), "{good:?}");
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn exported_script_applies_the_same_changes_as_an_apply() {
    let Some(container) =
        enabled_with_mongosh("exported_script_applies_the_same_changes_as_an_apply")
    else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst, copy) = (dbs.ns("src"), dbs.ns("dst"), dbs.ns("dst_copy"));
    let docs = |v: i32| {
        (1..=5)
            .map(|i| doc! { "_id": i, "v": v, "nested": { "a": [i, "x"] } })
            .collect::<Vec<_>>()
    };
    seed(&client, &src, docs(1)).await;
    // 1,2 missing; 3 differs; 4,5 equal; 6,7 extra
    let mut target = vec![doc! { "_id": 3, "v": 99 }];
    target.extend(docs(1).into_iter().skip(3));
    target.extend([doc! { "_id": 6, "v": 1 }, doc! { "_id": 7, "v": 1 }]);
    seed(&client, &dst, target.clone()).await;
    seed(&client, &copy, target).await;
    let (p_script, p_apply) = (
        pair(1, src.clone(), dst.clone(), &["_id"]),
        pair(2, src.clone(), copy.clone(), &["_id"]),
    );
    open(&mut rt, &config).await;
    let first = compare(&mut rt, 1, vec![p_script.clone(), p_apply]).await;
    assert!(first.iter().all(|(_, s)| summary_tuple(s) == (2, 2, 2, 1)));
    let ops = all_ops(&mut rt, 1).await;
    assert_eq!(ops.len(), 5);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.js");
    let outcome = export_ok(&mut rt, 2, script_request(ops.clone()), &path).await;
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (5, 0, 0)
    );
    let script = read(&path);
    assert!(script.starts_with("// complete export"), "{script}");
    assert!(!script.contains("quit(1)"));
    let before = all_docs(&client, &dst).await;
    assert_eq!(before.len(), 5, "the export itself wrote nothing");
    assert_eq!(all_docs(&client, &copy).await, before);

    let run = run_script(&container, &dbs.db, &script);
    assert!(run.succeeded() && !run.printed_error(), "{run:?}");
    applied_equals(&client, &mut rt, &dst, &copy, &src, ops).await;
    for (_, s) in compare(&mut rt, 4, vec![p_script]).await {
        assert_eq!(summary_tuple(&s), (5, 0, 0, 0), "the script converged");
    }
}

async fn applied_equals(
    client: &mongodb::Client,
    rt: &mut curlybrace_core::runtime::RuntimeHandle,
    scripted: &Namespace,
    applied: &Namespace,
    source: &Namespace,
    ops: Vec<curlybrace_core::compare::apply::ApplyOp>,
) {
    let report = apply(rt, 3, for_pair(&ops, 2), IdPolicy::Keep).await;
    assert_eq!(report.reports[0].1.applied, 5, "{:?}", report.reports);
    let by_script = all_docs(client, scripted).await;
    assert_eq!(by_script, all_docs(client, applied).await);
    assert_eq!(by_script, all_docs(client, source).await);
}

/// One document with a value of every BSON type the shell can express.
fn every_type() -> Document {
    let nested = doc! { "l1": { "l2": { "l3": [1, { "l4": ["deep", [true, Bson::Null]] }] } } };
    doc! {
        "_id": 1,
        "int32": 42_i32,
        "int32_min": i32::MIN,
        "int64_big": 9_007_199_254_740_993_i64,
        "int64_min": i64::MIN,
        "double_frac": 1.5_f64,
        "double_integral": 1.0_f64,
        "double_neg_zero": -0.0_f64,
        "double_nan": f64::NAN,
        "double_inf": f64::INFINITY,
        "double_ninf": f64::NEG_INFINITY,
        "double_tiny": 5e-324_f64,
        "decimal": "1234567890.123456789".parse::<Decimal128>().unwrap(),
        "decimal_nan": "NaN".parse::<Decimal128>().unwrap(),
        "string": "caf\u{e9} \u{2028}\u{2029} `tick` ${x} \"q\" \\ \n\t",
        "empty_string": "",
        "bool_true": true,
        "bool_false": false,
        "null": Bson::Null,
        "date_now": DateTime::from_millis(1_700_000_000_123),
        "date_epoch": DateTime::from_millis(0),
        "date_pre_1970": DateTime::from_millis(-1),
        "date_1900": DateTime::from_millis(-2_208_988_800_000),
        "date_year_1": DateTime::from_millis(-62_135_596_800_000),
        "date_year_10000": DateTime::from_millis(253_402_300_800_000),
        "oid": ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap(),
        "bin_generic": Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 1, 2, 254, 255] },
        "bin_uuid": Binary { subtype: BinarySubtype::Uuid, bytes: (0..16).collect() },
        "bin_uuid_old": Binary { subtype: BinarySubtype::UuidOld, bytes: (16..32).collect() },
        "bin_md5": Binary { subtype: BinarySubtype::Md5, bytes: vec![7; 16] },
        "bin_user": Binary { subtype: BinarySubtype::UserDefined(128), bytes: vec![9, 9] },
        "bin_empty": Binary { subtype: BinarySubtype::Generic, bytes: vec![] },
        "regex": Regex { pattern: "^a/b\\d+$".into(), options: "imsx".into() },
        "regex_plain": Regex { pattern: "plain".into(), options: String::new() },
        "timestamp": Timestamp { time: 1_700_000_000, increment: 7 },
        "min_key": Bson::MinKey,
        "max_key": Bson::MaxKey,
        "code": Bson::JavaScriptCode("function () { return 1; }".into()),
        "code_scope": Bson::JavaScriptCodeWithScope(bson::JavaScriptCodeWithScope {
            code: "x + y".into(),
            scope: doc! { "x": 1, "y": "two" },
        }),
        "symbol": Bson::Symbol("sym".into()),
        "array": [1, "two", [3.5], { "k": "v" }, Bson::Null],
        "nested": nested,
        "a b": "name with a space",
        "__proto__": "own field",
    }
}

/// A DBPointer cannot be built through the typed API: decode it from raw bytes.
fn db_pointer_doc() -> Document {
    let ns = b"db.coll";
    let mut body = Vec::new();
    body.extend([0x10]);
    body.extend(b"_id\0");
    body.extend(3_i32.to_le_bytes());
    body.push(0x0C);
    body.extend(b"p\0");
    body.extend(((ns.len() + 1) as i32).to_le_bytes());
    body.extend(ns);
    body.push(0);
    body.extend([7_u8; 12]);
    body.push(0);
    let mut bytes = ((body.len() + 4) as i32).to_le_bytes().to_vec();
    bytes.extend(body);
    let d = Document::from_reader(&bytes[..]).unwrap();
    assert!(matches!(d.get("p"), Some(Bson::DbPointer(_))), "{d:?}");
    d
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn script_round_trips_every_bson_type() {
    let Some(container) = enabled_with_mongosh("script_round_trips_every_bson_type") else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("types_src"), dbs.ns("types_dst"));
    let unsupported = [doc! { "_id": 2, "u": Bson::Undefined }, db_pointer_doc()];
    let mut docs = vec![every_type()];
    docs.extend(unsupported.clone());
    // The server may refuse the deprecated types; say so instead of guessing.
    let coll = client
        .database(&src.db)
        .collection::<Document>(&src.collection);
    for d in &docs {
        coll.insert_one(d.clone()).await.unwrap_or_else(|e| {
            panic!(
                "this server cannot store {:?}: {e}",
                d.keys().collect::<Vec<_>>()
            )
        });
    }
    seed(&client, &dst, vec![]).await;
    let p = pair(1, src.clone(), dst.clone(), &["_id"]);
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![p.clone()]).await;
    let ops = all_ops(&mut rt, 1).await;
    assert_eq!(ops.len(), 3);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("types.js");
    let outcome = export_ok(&mut rt, 2, script_request(ops), &path).await;
    // Undefined and DBPointer have no shell constructor: listed as skipped, never written.
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (1, 2, 0),
        "{outcome:?}"
    );
    let script = read(&path);
    assert_eq!(script.matches("bulkWrite(").count(), 1);
    assert!(script.contains("skipped"), "{script}");

    let run = run_script(&container, &dbs.db, &script);
    assert!(run.succeeded() && !run.printed_error(), "{run:?}");

    let (want, got) = (every_type(), only_doc(&client, &dst, 1).await);
    for (name, _) in want.iter() {
        assert_eq!(
            field_bytes(&got, name),
            field_bytes(&want, name),
            "field {name:?} differs: {:?} vs {:?}",
            got.get(name),
            want.get(name)
        );
    }
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        count(&client, &dst).await,
        1,
        "the unsupported documents were not copied"
    );

    // Compare again: the copied document is equal; only the skipped two are still source-only.
    let again = compare(&mut rt, 3, vec![p]).await;
    assert_eq!(summary_tuple(&again[0].1), (1, 2, 0, 0));
}

async fn only_doc(client: &mongodb::Client, ns: &Namespace, id: i32) -> Document {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find_one(doc! { "_id": id })
        .await
        .unwrap()
        .expect("the document")
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn script_with_hostile_names_and_values_runs_without_side_effects() {
    let Some(container) =
        enabled_with_mongosh("script_with_hostile_names_and_values_runs_without_side_effects")
    else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let src = dbs.ns("s\"); db.dropDatabase(); //");
    let dst = dbs.ns("t'); db.dropDatabase(); /* `x` ;");
    let sentinel = dbs.ns("sentinel");
    let other_db_sentinel = Namespace::new(&dbs.unicode_db, "sentinel");
    let evil_id = "id\"); db.dropDatabase(); //";
    let evil_field = "k\"]); db.dropDatabase(); //";
    let docs = vec![
        doc! { "_id": evil_id, evil_field: "v", "s": "a\u{2028}b\u{2029}c `d` ${x} </script> */ // \0 end" },
        doc! { "_id": 2, "s": "'; db.dropDatabase(); '" },
    ];
    seed(&client, &src, docs).await;
    seed(&client, &dst, vec![]).await;
    seed(&client, &sentinel, vec![doc! { "_id": "keep" }]).await;
    seed(&client, &other_db_sentinel, vec![doc! { "_id": "keep" }]).await;
    open(&mut rt, &config).await;
    compare(
        &mut rt,
        1,
        vec![pair(1, src.clone(), dst.clone(), &["_id"])],
    )
    .await;
    let ops = all_ops(&mut rt, 1).await;
    let mut req = script_request(ops);
    req.label = "evil\"\n*/ db.dropDatabase(); // `${x}` \u{2028} db.dropDatabase();".into();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hostile.js");
    let outcome = export_ok(&mut rt, 2, req, &path).await;
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (2, 0, 0)
    );
    let script = read(&path);
    for line in script.lines() {
        let t = line.trim_start();
        assert!(!t.starts_with("db.dropDatabase"), "{line}");
        assert!(!t.starts_with("*/"), "{line}");
    }
    assert!(!script.contains('\u{2028}') && !script.contains('\u{2029}'));

    let before: Vec<String> = collection_names(&client, &dbs.db).await;
    let run = run_script(&container, &dbs.db, &script);
    assert!(run.succeeded() && !run.printed_error(), "{run:?}");

    let mut after = collection_names(&client, &dbs.db).await;
    let mut expected = before;
    expected.sort();
    after.sort();
    assert_eq!(after, expected, "no collection appeared or vanished");
    assert_eq!(
        all_docs(&client, &sentinel).await,
        vec![doc! { "_id": "keep" }]
    );
    assert_eq!(
        all_docs(&client, &other_db_sentinel).await,
        vec![doc! { "_id": "keep" }]
    );
    let (want, got) = (all_docs(&client, &src).await, all_docs(&client, &dst).await);
    assert_eq!(want.len(), 2);
    for (w, g) in want.iter().zip(&got) {
        assert_eq!(bson::to_vec(w).unwrap(), bson::to_vec(g).unwrap());
    }
    assert_eq!(want.len(), got.len());
}

async fn collection_names(client: &mongodb::Client, db: &str) -> Vec<String> {
    client.database(db).list_collection_names().await.unwrap()
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn field_ops_in_a_script_update_only_the_chosen_fields() {
    let Some(container) =
        enabled_with_mongosh("field_ops_in_a_script_update_only_the_chosen_fields")
    else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst, copy) = (dbs.ns("f_src"), dbs.ns("f_dst"), dbs.ns("f_copy"));
    seed(
        &client,
        &src,
        vec![
            doc! { "_id": 1, "a": 1, "b": { "x": 1, "y": 1 }, "c": 1, "arr": [10, 20] },
            doc! { "_id": 2, "z": 1 },
        ],
    )
    .await;
    let target = vec![
        doc! { "_id": 1, "a": 2, "b": { "x": 2, "y": 2 }, "c": 2, "d": "only-target", "arr": [10, 99] },
        doc! { "_id": 2, "z": 1 },
    ];
    seed(&client, &dst, target.clone()).await;
    seed(&client, &copy, target.clone()).await;
    open(&mut rt, &config).await;
    compare(
        &mut rt,
        1,
        vec![
            pair(1, src.clone(), dst.clone(), &["_id"]),
            pair(2, src, copy.clone(), &["_id"]),
        ],
    )
    .await;
    let paths = vec![
        vec![f("a")],
        vec![f("b"), f("x")],
        vec![f("d")],
        vec![f("arr"), PathSegment::Index(1)],
    ];
    let field_op = |pair| fields_op(pair, Bson::Int32(1), paths.clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fields.js");
    let outcome = export_ok(&mut rt, 2, script_request(vec![field_op(1)]), &path).await;
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (1, 0, 0),
        "{outcome:?}"
    );
    let script = read(&path);
    assert!(
        script.contains("updateOne") && !script.contains("replaceOne"),
        "{script}"
    );
    let run = run_script(&container, &dbs.db, &script);
    assert!(run.succeeded() && !run.printed_error(), "{run:?}");

    let after = all_docs(&client, &dst).await;
    assert_eq!(
        after[0],
        doc! { "_id": 1, "a": 1, "b": { "x": 1, "y": 2 }, "c": 2, "arr": [10, 20] }
    );
    assert_eq!(after[1], target[1], "the neighbour is untouched");
    let report = apply(&mut rt, 3, vec![field_op(2)], IdPolicy::Keep).await;
    assert_eq!(report.reports[0].1.applied, 1);
    assert_eq!(
        after,
        all_docs(&client, &copy).await,
        "same result as an in-process apply"
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_script_cut_short_never_runs() {
    let Some(container) = enabled_with_mongosh("a_script_cut_short_never_runs") else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("cut_src"), dbs.ns("cut_dst"));
    let docs: Vec<Document> = (1..=6)
        .map(|i| doc! { "_id": i, "v": format!("value {i}") })
        .collect();
    seed(&client, &src, docs).await;
    seed(&client, &dst, vec![]).await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst.clone(), &["_id"])]).await;
    let ops = all_ops(&mut rt, 1).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cut.js");
    export_ok(&mut rt, 2, script_request(ops), &path).await;
    let script = read(&path);
    // The one statement ends with the catch block's `quit(2)` call; every cut before that final `)`
    // leaves an unfinished expression, which the shell never evaluates.
    let end = script.rfind(')').expect("the closing parenthesis");
    let first_line = script.find('\n').unwrap() + 1;
    let mut offsets = vec![0, 1, first_line, first_line - 1];
    offsets.extend((1..8).map(|n| first_line + (end - first_line) * n / 8));
    offsets.extend([end - 1, end]);
    for mut at in offsets {
        while !script.is_char_boundary(at) {
            at -= 1;
        }
        let run = run_script(&container, &dbs.db, &script[..at]);
        assert_eq!(count(&client, &dst).await, 0, "cut at {at}: {run:?}");
    }

    // A script that is still being written starts with `quit(1);`: nothing runs, non-zero exit.
    let complete_line_len = INCOMPLETE_LINE.len();
    assert!(
        script[..complete_line_len]
            .trim_end()
            .starts_with("// complete export")
    );
    let incomplete = format!("{INCOMPLETE_LINE}{}", &script[complete_line_len..]);
    assert!(incomplete.starts_with("quit(1);"));
    let run = run_script(&container, &dbs.db, &incomplete);
    assert_ne!(run.code, Some(0), "{run:?}");
    assert_eq!(count(&client, &dst).await, 0, "{run:?}");

    // Control: the complete script does change the database.
    let run = run_script(&container, &dbs.db, &script);
    assert!(run.succeeded() && !run.printed_error(), "{run:?}");
    assert_eq!(count(&client, &dst).await, 6);
    send(
        &rt,
        curlybrace_core::compare::protocol::CompareCommand::Close { session: SESSION },
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn stale_documents_are_listed_as_skipped_and_not_exported_as_writes() {
    let Some(container) =
        enabled_with_mongosh("stale_documents_are_listed_as_skipped_and_not_exported_as_writes")
    else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("stale_src"), dbs.ns("stale_dst"));
    seed(
        &client,
        &src,
        vec![
            doc! { "_id": 1, "v": "fresh insert" },
            doc! { "_id": 2, "v": "STALE-SOURCE-VALUE" },
        ],
    )
    .await;
    seed(
        &client,
        &dst,
        vec![
            doc! { "_id": 2, "v": "old" },
            doc! { "_id": 6, "v": "doomed" },
        ],
    )
    .await;
    open(&mut rt, &config).await;
    compare(&mut rt, 1, vec![pair(1, src, dst.clone(), &["_id"])]).await;
    let ops = all_ops(&mut rt, 1).await;
    assert_eq!(ops.len(), 3);
    // A second client changes both target documents after the compare.
    let coll = client
        .database(&dst.db)
        .collection::<Document>(&dst.collection);
    coll.update_one(doc! { "_id": 2 }, doc! { "$set": { "v": "changed" } })
        .await
        .unwrap();
    coll.update_one(doc! { "_id": 6 }, doc! { "$set": { "v": "also changed" } })
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stale.js");
    let outcome = export_ok(&mut rt, 2, script_request(ops), &path).await;
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (1, 2, 0),
        "{outcome:?}"
    );
    let script = read(&path);
    assert_eq!(script.matches("insertOne").count(), 1);
    assert_eq!(script.matches("replaceOne").count(), 0, "{script}");
    assert_eq!(script.matches("deleteOne").count(), 0, "{script}");
    assert!(
        !script.contains("STALE-SOURCE-VALUE"),
        "a skipped replace carries no document"
    );
    assert_eq!(script.matches("// skipped").count(), 2, "{script}");

    let run = run_script(&container, &dbs.db, &script);
    assert!(run.succeeded() && !run.printed_error(), "{run:?}");
    assert_eq!(
        raw_docs(&client, &dst).await,
        vec![
            doc! { "_id": 1, "v": "fresh insert" },
            doc! { "_id": 2, "v": "changed" },
            doc! { "_id": 6, "v": "also changed" }
        ]
    );
}

fn key_order(d: &Document) -> Vec<String> {
    d.keys().cloned().collect()
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn javascript_key_order_is_kept_or_the_document_is_skipped() {
    let Some(container) =
        enabled_with_mongosh("javascript_key_order_is_kept_or_the_document_is_skipped")
    else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src, dst) = (dbs.ns("order_src"), dbs.ns("order_dst"));
    // JavaScript lists canonical array-index keys ("2", "10") first, ascending, whatever the
    // literal order: a document whose BSON order differs is skipped, never written reordered.
    // "01", "-1", "4294967295" are not array indexes and keep their place.
    let docs = vec![
        doc! { "_id": 1, "b": 1, "10": 2, "2": 3 },
        doc! { "_id": 2, "n": { "2": 1, "10": 2, "b": 3, "01": 4, "-1": 5, "4294967295": 6 } },
        doc! { "_id": 3, "n": { "b": 1, "2": 2 } },
        doc! { "_id": { "2": 1, "b": 2 }, "v": "sub-document id, in order" },
        doc! { "_id": { "b": 1, "2": 2 }, "v": "sub-document id, out of order" },
    ];
    seed(&client, &src, docs.clone()).await;
    seed(&client, &dst, vec![]).await;
    open(&mut rt, &config).await;
    compare(
        &mut rt,
        1,
        vec![pair(1, src.clone(), dst.clone(), &["_id"])],
    )
    .await;
    let ops = all_ops(&mut rt, 1).await;
    assert_eq!(ops.len(), 5);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("order.js");
    let outcome = export_ok(&mut rt, 2, script_request(ops.clone()), &path).await;
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (2, 3, 0),
        "{outcome:?}"
    );
    let script = read(&path);
    assert_eq!(script.matches("// skipped").count(), 3, "{script}");
    // The JSON plan refuses exactly the same operations (exported before the script changes the target).
    let plan = dir.path().join("order.json");
    let outcome = export_ok(&mut rt, 3, request(ExportFormat::JsonPlan, ops), &plan).await;
    assert_eq!((outcome.written, outcome.skipped), (2, 3), "{outcome:?}");
    let run = run_script(&container, &dbs.db, &script);
    assert!(run.succeeded() && !run.printed_error(), "{run:?}");

    let got = all_docs(&client, &dst).await;
    assert_eq!(got.len(), 2, "{got:?}");
    let nested = got
        .iter()
        .find(|d| d.get_i32("_id") == Ok(2))
        .expect("doc 2");
    assert_eq!(
        key_order(nested.get_document("n").unwrap()),
        ["2", "10", "b", "01", "-1", "4294967295"]
    );
    let sub = got
        .iter()
        .find_map(|d| d.get_document("_id").ok())
        .expect("the sub-document id");
    assert_eq!(key_order(sub), ["2", "b"]);
    assert!(
        got.iter()
            .all(|d| d.get_i32("_id") != Ok(1) && d.get_i32("_id") != Ok(3))
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
async fn a_failing_statement_stops_later_statements_and_exits_non_zero() {
    let Some(container) =
        enabled_with_mongosh("a_failing_statement_stops_later_statements_and_exits_non_zero")
    else {
        return;
    };
    let (dbs, client, mut rt, config) = local_runtime().await;
    let (src1, dst1) = (dbs.ns("fail_src1"), dbs.ns("fail_dst1"));
    let (src2, dst2) = (dbs.ns("fail_src2"), dbs.ns("fail_dst2"));
    let user = |id: i32, e: &str| doc! { "_id": id, "email": e };
    seed(
        &client,
        &src1,
        vec![
            user(10, "ok@x"),
            user(11, "TAKEN-SECRET@x"),
            user(12, "ok2@x"),
        ],
    )
    .await;
    seed(&client, &dst1, vec![user(1, "TAKEN-SECRET@x")]).await;
    client
        .database(&dst1.db)
        .collection::<Document>(&dst1.collection)
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
    seed(&client, &src2, vec![user(20, "later@x")]).await;
    seed(&client, &dst2, vec![]).await;
    open(&mut rt, &config).await;
    compare(
        &mut rt,
        1,
        vec![
            pair(1, src1, dst1.clone(), &["_id"]),
            pair(2, src2, dst2.clone(), &["_id"]),
        ],
    )
    .await;
    let mut ops = all_ops(&mut rt, 1).await;
    ops.extend(all_ops(&mut rt, 2).await);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fail.js");
    let outcome = export_ok(&mut rt, 2, script_request(ops), &path).await;
    assert_eq!(outcome.written, 5, "{outcome:?}");
    let script = read(&path);

    let run = run_script(&container, &dbs.db, &script);
    assert_eq!(run.code, Some(2), "{run:?}");
    assert!(run.output().contains("a bulkWrite failed"), "{run:?}");
    assert!(
        !run.output().contains("TAKEN-SECRET"),
        "the notice carries no values: {run:?}"
    );
    // The unordered statement applied what it could; the next statement did not run.
    assert_eq!(
        count(&client, &dst1).await,
        2,
        "ok@x and ok2@x landed, the old row was deleted"
    );
    assert_eq!(
        count(&client, &dst2).await,
        0,
        "the later statement did not run"
    );
}
