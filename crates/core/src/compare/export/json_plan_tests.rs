use bson::{Bson, doc};
use serde_json::{Value, json};

use super::*;
use crate::compare::apply::{Direction, IdPolicy, SkipReason};
use crate::compare::diff::PathSegment::{Field, Index};
use crate::compare::export::ExportCounts;
use crate::compare::fingerprint::Fingerprint;
use crate::compare::key::KeyTuple;

fn header(label: &str) -> ExportHeader {
    ExportHeader {
        label: label.to_string(),
        counts: ExportCounts {
            insert: 1,
            replace: 2,
            delete: 3,
            update_fields: 4,
            skipped: 5,
        },
        id_policy: IdPolicy::Generate,
        count_width: 1,
        complete: true,
        destination: None,
    }
}

fn pair(db: &str) -> PlanPair {
    PlanPair {
        source: Namespace::new(db, "items"),
        target: Namespace::new("tdb", "titems"),
        key: vec!["sku".to_string(), "a.b".to_string()],
    }
}

fn meta(direction: Direction, key: Vec<Bson>, source: Option<u8>, target: Option<u8>) -> OpMeta {
    OpMeta {
        direction,
        key: KeyTuple(key),
        source: source.map(|b| Fingerprint([b; 16])),
        target: target.map(|b| Fingerprint([b; 16])),
    }
}

fn insert_op() -> PlanOp {
    PlanOp::Write {
        meta: meta(
            Direction::SourceToTarget,
            vec![Bson::Int32(1)],
            Some(0xab),
            None,
        ),
        op: ShellOp::Insert {
            doc: doc! { "_id": 1_i64, "d": 2.5, "when": bson::DateTime::from_millis(0) },
        },
    }
}

fn write(ops_per_pair: &[Vec<PlanOp>], label: &str) -> String {
    let mut w = PlanWriter::new(Vec::new());
    w.begin(&header(label)).expect("begin");
    for (i, ops) in ops_per_pair.iter().enumerate() {
        w.pair(&pair(&format!("db{i}"))).expect("pair");
        for op in ops {
            w.operation(op).expect("operation");
        }
    }
    String::from_utf8(w.finish().expect("finish")).expect("utf8")
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("invalid JSON ({e}):\n{text}"))
}

#[test]
fn header_fields() {
    let v = parse(&write(&[], "Nightly"));
    assert_eq!(
        v["header"],
        json!({
            "format": "curlybrace-compare-plan",
            "version": 1,
            "label": "Nightly",
            "complete": true,
            "counts": { "insert": 1, "replace": 2, "delete": 3, "updateFields": 4, "skipped": 5 },
            "idPolicy": "generate"
        })
    );
    assert_eq!(v["pairs"], json!([]));
}

#[test]
fn zero_one_and_many_operations_parse() {
    let zero = parse(&write(&[vec![]], "l"));
    assert_eq!(zero["pairs"][0]["operations"], json!([]));

    let one = parse(&write(&[vec![insert_op()]], "l"));
    assert_eq!(
        one["pairs"][0]["operations"].as_array().map(Vec::len),
        Some(1)
    );

    let many = parse(&write(
        &[
            vec![insert_op(), insert_op(), insert_op()],
            vec![],
            vec![insert_op()],
        ],
        "l",
    ));
    let pairs = many["pairs"].as_array().expect("pairs");
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[0]["operations"].as_array().map(Vec::len), Some(3));
    assert_eq!(pairs[1]["operations"].as_array().map(Vec::len), Some(0));
    assert_eq!(pairs[2]["operations"].as_array().map(Vec::len), Some(1));
}

#[test]
fn pair_shape() {
    let v = parse(&write(&[vec![]], "l"));
    assert_eq!(
        v["pairs"][0],
        json!({
            "source": { "db": "db0", "collection": "items" },
            "target": { "db": "tdb", "collection": "titems" },
            "key": ["sku", "a.b"],
            "operations": []
        })
    );
}

#[test]
fn insert_carries_canonical_extended_json() {
    let v = parse(&write(&[vec![insert_op()]], "l"));
    assert_eq!(
        v["pairs"][0]["operations"][0],
        json!({
            "op": "insert",
            "direction": "sourceToTarget",
            "destination": "target",
            "key": [{ "$numberInt": "1" }],
            "sourceFingerprint": "ab".repeat(16),
            "targetFingerprint": null,
            "document": {
                "_id": { "$numberLong": "1" },
                "d": { "$numberDouble": "2.5" },
                "when": { "$date": { "$numberLong": "0" } }
            }
        })
    );
}

#[test]
fn replace_delete_and_update_shapes() {
    let ops = vec![
        PlanOp::Write {
            meta: meta(
                Direction::TargetToSource,
                vec![Bson::String("k".into())],
                Some(1),
                Some(2),
            ),
            op: ShellOp::Replace {
                id: Bson::Int32(2),
                doc: doc! { "_id": 2 },
            },
        },
        PlanOp::Write {
            meta: meta(
                Direction::SourceToTarget,
                vec![Bson::Int32(3)],
                None,
                Some(2),
            ),
            op: ShellOp::Delete { id: Bson::Int32(3) },
        },
        PlanOp::Write {
            meta: meta(
                Direction::SourceToTarget,
                vec![Bson::Int32(4)],
                Some(1),
                Some(2),
            ),
            op: ShellOp::UpdateFields {
                id: Bson::Int32(4),
                set: vec![(
                    vec![Field("a".into()), Index(2), Field("b".into())],
                    Bson::Int64(5),
                )],
                unset: vec![vec![Field("c".into())]],
            },
        },
    ];
    let v = parse(&write(&[ops], "l"));
    let o = &v["pairs"][0]["operations"];
    assert_eq!(o[0]["op"], "replace");
    assert_eq!(o[0]["direction"], "targetToSource");
    assert_eq!(o[0]["key"], json!(["k"]));
    assert_eq!(o[0]["id"], json!({ "$numberInt": "2" }));
    assert_eq!(o[0]["document"], json!({ "_id": { "$numberInt": "2" } }));
    assert_eq!(o[0]["sourceFingerprint"], "01".repeat(16));
    assert_eq!(o[0]["targetFingerprint"], "02".repeat(16));
    assert_eq!(o[1]["op"], "delete");
    assert_eq!(o[1]["id"], json!({ "$numberInt": "3" }));
    assert_eq!(o[1]["sourceFingerprint"], Value::Null);
    assert!(o[1].get("document").is_none());
    assert_eq!(o[2]["op"], "updateFields");
    assert_eq!(
        o[2]["set"],
        json!([{ "path": ["a", 2, "b"], "value": { "$numberLong": "5" } }])
    );
    assert_eq!(o[2]["unset"], json!([["c"]]));
}

#[test]
fn paths_are_arrays_of_strings_and_numbers_never_dotted() {
    let op = PlanOp::Write {
        meta: meta(
            Direction::SourceToTarget,
            vec![Bson::Int32(1)],
            Some(1),
            Some(2),
        ),
        op: ShellOp::UpdateFields {
            id: Bson::Int32(1),
            set: vec![(vec![Field("a b".into()), Index(0)], Bson::Null)],
            unset: vec![],
        },
    };
    let v = parse(&write(&[vec![op]], "l"));
    let path = &v["pairs"][0]["operations"][0]["set"][0]["path"];
    assert_eq!(path, &json!(["a b", 0]));
    assert!(path[1].is_u64());
}

#[test]
fn skipped_operations_carry_their_reason_and_never_a_document() {
    let ops = vec![
        PlanOp::Skipped {
            meta: meta(
                Direction::SourceToTarget,
                vec![Bson::Int32(1)],
                Some(1),
                Some(2),
            ),
            kind: ApplyKind::Replace,
            reason: SkipReason::Stale.into(),
        },
        PlanOp::Skipped {
            meta: meta(
                Direction::TargetToSource,
                vec![Bson::Int32(2)],
                Some(1),
                None,
            ),
            kind: ApplyKind::Fields,
            reason: SkipReason::NotPlanned.into(),
        },
    ];
    let v = parse(&write(&[ops], "l"));
    let o = &v["pairs"][0]["operations"];
    assert_eq!(o[0]["op"], "skipped");
    assert_eq!(o[0]["kind"], "replace");
    assert_eq!(o[0]["reason"], "stale");
    assert_eq!(o[1]["kind"], "updateFields");
    assert_eq!(o[1]["reason"], "notPlanned");
    for entry in o.as_array().expect("array") {
        assert!(entry.get("document").is_none() && entry.get("set").is_none());
        assert!(entry.get("direction").is_some() && entry.get("key").is_some());
    }
}

#[test]
fn a_hostile_label_and_names_are_escaped() {
    let label = "evil\"}, \"pairs\": [] //\n\u{2028}\u{0}</script>";
    let text = write(&[vec![]], label);
    let v = parse(&text);
    assert_eq!(v["header"]["label"], label);
    assert!(v["pairs"].as_array().is_some_and(|p| p.len() == 1));
    assert!(!text.contains("\u{2028}"));
}

#[test]
fn a_field_order_in_documents_is_preserved() {
    let op = PlanOp::Write {
        meta: meta(
            Direction::SourceToTarget,
            vec![Bson::Int32(1)],
            Some(1),
            None,
        ),
        op: ShellOp::Insert {
            doc: doc! { "z": 1, "a": 2, "m": 3 },
        },
    };
    let text = write(&[vec![op]], "l");
    let z = text.find("\"z\"").expect("z");
    let a = text.find("\"a\"").expect("a");
    let m = text.find("\"m\"").expect("m");
    assert!(z < a && a < m);
}

fn insert_of(value: Bson) -> PlanOp {
    PlanOp::Write {
        meta: meta(
            Direction::SourceToTarget,
            vec![Bson::Int32(1)],
            Some(1),
            None,
        ),
        op: ShellOp::Insert {
            doc: doc! { "nested": { "v": [value] } },
        },
    }
}

fn operation_result(op: &PlanOp) -> (Result<(), RenderError>, String) {
    let mut w = PlanWriter::new(Vec::new());
    w.begin(&header("l")).expect("begin");
    w.pair(&pair("d")).expect("pair");
    let before = w.inner.len();
    let r = w.operation(op);
    assert_eq!(
        w.inner.len() == before,
        r.is_err(),
        "an error writes nothing"
    );
    (r, String::from_utf8(w.inner.clone()).expect("utf8"))
}

#[test]
fn values_the_script_cannot_express_are_refused_by_the_plan_too() {
    let pointer = Bson::try_from(json!({
        "$dbPointer": { "$ref": "c", "$id": { "$oid": "507f1f77bcf86cd799439011" } }
    }))
    .expect("a db pointer");
    let bad_regex = Bson::RegularExpression(bson::Regex {
        pattern: "a".into(),
        options: "g".into(),
    });
    for value in [Bson::Undefined, pointer, bad_regex] {
        let (r, text) = operation_result(&insert_of(value));
        assert_eq!(r, Err(RenderError::Unrenderable));
        assert!(!text.contains("$undefined") && !text.contains("dbPointer"));
    }
    let (r, _) = operation_result(&insert_of(Bson::Int32(1)));
    assert_eq!(r, Ok(()));
}

#[test]
fn ids_and_set_values_are_checked_as_well() {
    let m = meta(
        Direction::SourceToTarget,
        vec![Bson::Int32(1)],
        Some(1),
        Some(2),
    );
    let delete = PlanOp::Write {
        meta: m.clone(),
        op: ShellOp::Delete {
            id: Bson::Undefined,
        },
    };
    assert_eq!(operation_result(&delete).0, Err(RenderError::Unrenderable));
    let update = PlanOp::Write {
        meta: m,
        op: ShellOp::UpdateFields {
            id: Bson::Int32(1),
            set: vec![(vec![Field("a".into())], Bson::Undefined)],
            unset: vec![],
        },
    };
    assert_eq!(operation_result(&update).0, Err(RenderError::Unrenderable));
}

type UpdateCase = (
    Vec<(Vec<crate::compare::diff::PathSegment>, Bson)>,
    Vec<Vec<crate::compare::diff::PathSegment>>,
    bool,
);

#[test]
fn update_fields_the_script_refuses_are_refused_by_the_plan_too() {
    let one = || Bson::Int32(1);
    let f = |n: &str| Field(n.to_string());
    let cases: Vec<UpdateCase> = vec![
        (vec![(vec![f("a")], one())], vec![], false),
        (vec![(vec![f("a"), Index(2)], one())], vec![], false),
        (vec![], vec![], true),
        (vec![(vec![f("_id")], one())], vec![], true),
        (vec![(vec![f("a"), f("$x")], one())], vec![], true),
        (vec![(vec![f("")], one())], vec![], true),
        (vec![], vec![vec![f("a"), Index(1)]], true),
        (
            vec![(vec![f("a")], one()), (vec![f("a"), f("b")], one())],
            vec![],
            true,
        ),
    ];
    for (set, unset, refused) in cases {
        let op = PlanOp::Write {
            meta: meta(
                Direction::SourceToTarget,
                vec![Bson::Int32(1)],
                Some(1),
                Some(2),
            ),
            op: ShellOp::UpdateFields {
                id: Bson::Int32(1),
                set: set.clone(),
                unset: unset.clone(),
            },
        };
        let store = crate::mongo_write_fields::build_statement(&Bson::Int32(1), &set, &unset);
        assert_eq!(store.is_err(), refused, "{set:?} {unset:?}");
        assert_eq!(
            operation_result(&op).0.is_err(),
            refused,
            "{set:?} {unset:?}"
        );
    }
}

#[test]
fn calling_the_writer_out_of_order_is_a_sequence_error() {
    let mut w = PlanWriter::new(Vec::new());
    assert_eq!(w.pair(&pair("d")), Err(RenderError::Sequence));
    assert_eq!(w.operation(&insert_op()), Err(RenderError::Sequence));
    w.begin(&header("l")).expect("begin");
    assert_eq!(w.begin(&header("l")), Err(RenderError::Sequence));
    assert_eq!(w.operation(&insert_op()), Err(RenderError::Sequence));
    assert!(matches!(
        PlanWriter::new(Vec::new()).finish(),
        Err(RenderError::Sequence)
    ));
}

#[test]
fn an_empty_namespace_is_a_bad_name() {
    let mut w = PlanWriter::new(Vec::new());
    w.begin(&header("l")).expect("begin");
    let mut bad = pair("d");
    bad.target = Namespace::new("", "c");
    assert_eq!(w.pair(&bad), Err(RenderError::BadName));
}

fn header_len(complete: bool, width: usize, n: u64) -> usize {
    let mut h = header("same label");
    h.count_width = width;
    h.complete = complete;
    h.counts = ExportCounts {
        insert: n,
        replace: n,
        delete: n,
        update_fields: n,
        skipped: n,
    };
    let mut bytes = Vec::new();
    PlanWriter::new(&mut bytes).begin(&h).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text.contains("\"complete\":true "), complete);
    assert_eq!(text.contains("\"complete\":false"), !complete);
    text.len()
}

#[test]
fn the_header_has_the_same_length_incomplete_and_complete_for_any_total() {
    for total in [0u64, 9, 10, 1200] {
        let width = crate::compare::export::count_width(total);
        assert_eq!(
            header_len(false, width, 0),
            header_len(true, width, total),
            "total {total}"
        );
    }
}

#[test]
fn a_label_cannot_clobber_the_header_markers() {
    let labels = [
        "@@counts@@",
        "\"@@counts@@\"",
        "\"complete\":false",
        "a\"b\\c\"@@counts@@\"\\",
        "@@counts@@@@counts@@",
    ];
    for label in labels {
        let lens: Vec<usize> = [false, true]
            .into_iter()
            .map(|complete| {
                let mut h = header(label);
                h.complete = complete;
                let mut bytes = Vec::new();
                PlanWriter::new(&mut bytes).begin(&h).unwrap();
                let mut text = String::from_utf8(bytes).unwrap();
                let len = text.len();
                text.push_str("]}");
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(v["header"]["label"], label);
                assert!(v["header"]["counts"].is_object(), "{label}");
                assert_eq!(v["header"]["complete"], complete);
                len
            })
            .collect();
        assert_eq!(lens[0], lens[1], "{label}");
    }
}
