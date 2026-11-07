use bson::oid::ObjectId;
use bson::spec::BinarySubtype;
use bson::{Binary, Bson, DateTime, Document, Regex, doc};

use super::{Shape, from_profile_command, from_query};
use crate::store::Query;

fn shape_of(filter: Document) -> Shape {
    from_query(&Query {
        filter,
        ..Query::default()
    })
}

fn sorted(sort: Document) -> Shape {
    from_query(&Query {
        sort: Some(sort),
        ..Query::default()
    })
}

#[test]
fn same_shape_for_different_values() {
    let a = shape_of(doc! { "status": "open", "age": { "$gt": 18 } });
    let b = shape_of(doc! { "status": "closed", "age": { "$gt": 99 } });
    assert_eq!(a, b);
    assert_eq!(a.hash(), b.hash());
}

#[test]
fn different_operator_is_a_different_shape() {
    let a = shape_of(doc! { "age": { "$gt": 18 } });
    let b = shape_of(doc! { "age": { "$gte": 18 } });
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn field_order_in_filter_is_irrelevant() {
    let a = shape_of(doc! { "a": 1, "b": 2 });
    let b = shape_of(doc! { "b": 2, "a": 1 });
    assert_eq!(a, b);
}

#[test]
fn sort_order_is_significant() {
    let a = sorted(doc! { "a": 1, "b": -1 });
    let b = sorted(doc! { "b": -1, "a": 1 });
    let c = sorted(doc! { "a": -1, "b": -1 });
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.json(), r#"{"filter":{},"sort":{"a":1,"b":-1}}"#);
}

#[test]
fn in_arrays_ignore_arity_and_values() {
    let a = shape_of(doc! { "r": { "$in": ["eu"] } });
    let b = shape_of(doc! { "r": { "$in": ["us", "de", "fr"] } });
    assert_eq!(a, b);
    assert_eq!(a.json(), r#"{"filter":{"r":{"$in":["?"]}}}"#);
    let n = shape_of(doc! { "r": { "$nin": [1, 2] } });
    assert_eq!(n.json(), r#"{"filter":{"r":{"$nin":["?"]}}}"#);
}

#[test]
fn or_children_are_order_insensitive() {
    let a = shape_of(doc! { "$or": [{ "a": 1 }, { "b": { "$gt": 2 } }] });
    let b = shape_of(doc! { "$or": [{ "b": { "$gt": 7 } }, { "a": 5 }] });
    assert_eq!(a, b);
    let and = shape_of(doc! { "$and": [{ "z": 1 }, { "y": 1 }] });
    let and_swapped = shape_of(doc! { "$and": [{ "y": 1 }, { "z": 1 }] });
    assert_eq!(and, and_swapped);
}

#[test]
fn regex_pattern_is_masked() {
    let s = shape_of(doc! { "name": { "$regex": "^hunter2", "$options": "i" } });
    assert_eq!(
        s.json(),
        r#"{"filter":{"name":{"$options":"?","$regex":"?"}}}"#
    );
    let bson_regex = shape_of(doc! { "name": Bson::RegularExpression(Regex {
    pattern: "^hunter2".into(), options: "i".into() }) });
    assert_eq!(bson_regex.json(), r#"{"filter":{"name":"?"}}"#);
}

#[test]
fn nested_and_elemmatch_masked() {
    let s = shape_of(doc! {
        "items": { "$elemMatch": { "sku": "SECRET-7f3a", "qty": { "$gt": 918273645 } } },
        "$and": [{ "a": { "$in": [1, 2] } }],
    });
    assert_eq!(
        s.json(),
        r#"{"filter":{"$and":[{"a":{"$in":["?"]}}],"items":{"$elemMatch":{"qty":{"$gt":"?"},"sku":"?"}}}}"#
    );
}

#[test]
fn no_literal_value_survives() {
    let filter = doc! {
        "name": "SECRET-7f3a",
        "n": 918273645_i64,
        "pw": { "$regex": "^hunter2", "$options": "SECRET-opts" },
        "oid": ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap(),
        "at": DateTime::from_millis(1_700_000_123_456),
        "bin": Binary { subtype: BinarySubtype::Generic, bytes: b"SECRET-bytes".to_vec() },
        "items": { "$elemMatch": { "sku": "SECRET-7f3a", "tags": { "$in": ["SECRET-tag", 4242] } } },
        "$or": [{ "x": "SECRET-or" }, { "y": { "$ne": 31337.5 } }],
        "d": 6.62607,
    };
    let q = Query {
        filter,
        sort: Some(doc! { "createdAt": -1 }),
        projection: Some(doc! { "name": 1, "secret_field_value": 1 }),
        skip: 918273645,
        limit: 777,
    };
    let json = from_query(&q).json();
    for leak in [
        "SECRET",
        "918273645",
        "hunter2",
        "507f1f77",
        "1700000123456",
        "4242",
        "31337",
        "6.62607",
        "777",
        "U0VDUkVU",
    ] {
        assert!(!json.contains(leak), "{leak} leaked into {json}");
    }
}

#[test]
fn projection_keeps_names_and_mode() {
    let inc = from_query(&Query {
        projection: Some(doc! { "b": 1, "a": 1, "_id": 0 }),
        ..Query::default()
    });
    assert_eq!(
        inc.json(),
        r#"{"filter":{},"projection":{"include":["_id","a","b"]}}"#
    );
    let exc = from_query(&Query {
        projection: Some(doc! { "b": 0, "a": 0 }),
        ..Query::default()
    });
    assert_eq!(
        exc.json(),
        r#"{"filter":{},"projection":{"exclude":["a","b"]}}"#
    );
}

#[test]
fn hash_is_stable_across_runs() {
    let s = shape_of(doc! { "status": "open" });
    assert_eq!(s.json(), r#"{"filter":{"status":"?"}}"#);
    assert_eq!(s.hash().len(), 64);
    assert_eq!(
        s.hash(),
        "cf61ab4bdd1bcb57cef426444626a05b7681e4d1638286324bbb8f0f297ae635"
    );
}

#[test]
fn json_round_trips_and_rejects_non_shapes() {
    let s = from_query(&Query {
        filter: doc! { "a": { "$in": [1, 2] }, "$or": [{ "b": 1 }] },
        sort: Some(doc! { "z": -1, "a": 1 }),
        projection: Some(doc! { "a": 1 }),
        ..Query::default()
    });
    assert_eq!(Shape::from_json(&s.json()).unwrap(), s);
    for bad in [
        "not json",
        "[]",
        "{}",
        r#"{"filter":{"a":"SECRET"}}"#,
        r#"{"filter":{"a":5}}"#,
        r#"{"filter":{},"extra":{}}"#,
        r#"{"filter":{},"sort":{"a":"?"}}"#,
        r#"{"filter":{},"projection":{"include":"a"}}"#,
    ] {
        assert!(Shape::from_json(bad).is_err(), "{bad} accepted");
    }
}

fn json_of(op: &str, command: Document) -> String {
    from_profile_command(op, &command).unwrap().json()
}

#[test]
fn profile_find_aggregate_count_distinct_update_delete_findandmodify_commands_map() {
    let find = doc! { "find": "c", "filter": { "a": 1 }, "sort": { "b": -1 },
    "projection": { "c": 1 }, "limit": 5, "$db": "d", "lsid": { "id": 1 } };
    assert_eq!(
        json_of("query", find.clone()),
        r#"{"filter":{"a":"?"},"sort":{"b":-1},"projection":{"include":["c"]}}"#
    );
    assert_eq!(
        json_of("find", find),
        json_of(
            "query",
            doc! { "filter": { "a": 2 },
            "sort": { "b": -1 }, "projection": { "c": 1 } }
        )
    );
    let want = r#"{"filter":{"a":"?"}}"#;
    assert_eq!(
        json_of("count", doc! { "count": "c", "query": { "a": 1 } }),
        want
    );
    assert_eq!(
        json_of(
            "distinct",
            doc! { "distinct": "c", "key": "k", "query": { "a": 1 } }
        ),
        want
    );
    assert_eq!(
        json_of(
            "update",
            doc! { "update": "c", "updates": [{ "q": { "a": 1 }, "u": { "$set": { "x": 1 } } }] }
        ),
        want
    );
    assert_eq!(
        json_of(
            "update",
            doc! { "q": { "a": 1 }, "u": { "$set": { "x": 1 } } }
        ),
        want
    );
    assert_eq!(
        json_of(
            "remove",
            doc! { "delete": "c", "deletes": [{ "q": { "a": 1 }, "limit": 1 }] }
        ),
        want
    );
    assert_eq!(
        json_of("delete", doc! { "deletes": [{ "q": { "a": 1 } }] }),
        want
    );
    assert_eq!(
        json_of(
            "findAndModify",
            doc! { "findAndModify": "c", "query": { "a": 1 }, "sort": { "b": 1 }, "update": {} }
        ),
        r#"{"filter":{"a":"?"},"sort":{"b":1}}"#
    );
    assert_eq!(
        json_of(
            "aggregate",
            doc! { "aggregate": "c", "pipeline": [
            { "$match": { "a": 1 } }, { "$sort": { "b": 1 } }, { "$group": { "_id": "$x" } }] }
        ),
        r#"{"filter":{"a":"?"},"sort":{"b":1}}"#
    );
    assert_eq!(
        json_of(
            "aggregate",
            doc! { "pipeline": [{ "$match": { "a": 1 } }, { "$group": { "_id": "$x" } }] }
        ),
        want
    );
}

#[test]
fn profile_getmore_and_unsupported_ops_are_none() {
    let cmd = doc! { "getMore": 1_i64, "collection": "c" };
    for op in [
        "getmore", "getMore", "insert", "command", "commands", "", "bogus",
    ] {
        assert!(from_profile_command(op, &cmd).is_none(), "{op}");
    }
    assert!(
        from_profile_command(
            "aggregate",
            &doc! { "pipeline": [{ "$group": { "_id": 1 } }] }
        )
        .is_none()
    );
    assert!(from_profile_command("aggregate", &doc! {}).is_none());
    assert!(from_profile_command("update", &doc! { "updates": [] }).is_none());
    assert!(from_profile_command("delete", &doc! {}).is_none());
}
