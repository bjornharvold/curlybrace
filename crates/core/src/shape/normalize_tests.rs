use bson::oid::ObjectId;
use bson::spec::BinarySubtype;
use bson::{
    Binary, Bson, DateTime, Decimal128, Document, JavaScriptCodeWithScope, Regex, Timestamp, doc,
};

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

fn json_of_filter(filter: Document) -> String {
    shape_of(filter).json()
}

#[test]
fn literal_document_keys_never_appear() {
    let filters = [
        doc! { "meta": { "user-SECRET-7f3a": 1 } },
        doc! { "tags": { "$all": [{ "k-SECRET-7f3a": 1 }] } },
        doc! { "meta": { "$eq": { "user-SECRET-7f3a": 1 } } },
        doc! { "meta": { "$in": [{ "user-SECRET-7f3a": 1 }] } },
        doc! { "meta": { "$ne": { "user-SECRET-7f3a": 1 } } },
        doc! { "items": { "$elemMatch": { "meta": { "user-SECRET-7f3a": { "deep-SECRET-7f3a": 1 } } } } },
        doc! { "$expr": { "$eq": ["$a", { "user-SECRET-7f3a": 1 }] } },
        doc! { "meta": { "$gt": { "user-SECRET-7f3a": 1 } } },
        doc! { "meta": { "$gt": 1, "user-SECRET-7f3a": 1 } },
        doc! { "$or": [{ "meta": { "user-SECRET-7f3a": 1 } }] },
        doc! { "loc": { "$near": { "$geometry": { "user-SECRET-7f3a": [1, 2] } } } },
    ];
    for filter in filters {
        let json = json_of_filter(filter.clone());
        assert!(!json.contains("SECRET"), "{filter} leaked: {json}");
    }
}

#[test]
fn operator_documents_keep_their_operators() {
    assert_eq!(
        json_of_filter(doc! { "age": { "$lt": 9, "$gt": 5 }, "meta": { "k": 1 } }),
        r#"{"filter":{"age":{"$gt":"?","$lt":"?"},"meta":"?"}}"#
    );
    assert_eq!(
        json_of_filter(doc! { "n": { "$not": { "$gt": 5 } }, "t": { "$all": [1, 2] } }),
        r#"{"filter":{"n":{"$not":{"$gt":"?"}},"t":{"$all":["?"]}}}"#
    );
}

#[test]
fn elemmatch_operand_keeps_field_paths() {
    assert_eq!(
        json_of_filter(
            doc! { "items": { "$elemMatch": { "sku": "x", "qty": { "$gt": 1 },
            "$or": [{ "z": 1 }, { "a": 2 }] } } }
        ),
        r#"{"filter":{"items":{"$elemMatch":{"$or":[{"a":"?"},{"z":"?"}],"qty":{"$gt":"?"},"sku":"?"}}}}"#
    );
    assert_eq!(
        json_of_filter(doc! { "n": { "$elemMatch": { "$gt": 1, "$lt": 5 } } }),
        r#"{"filter":{"n":{"$elemMatch":{"$gt":"?","$lt":"?"}}}}"#
    );
}

#[test]
fn from_json_rejects_literal_documents_with_data_keys() {
    for bad in [
        r#"{"filter":{"meta":{"user-1":"?"}}}"#,
        r#"{"filter":{"t":{"$all":[{"k":"?"}]}}}"#,
        r#"{"filter":{"m":{"$eq":{"user-1":"?"}}}}"#,
        r#"{"filter":{"m":{"$gt":"?","user-1":"?"}}}"#,
        r#"{"filter":{"m":{}}}"#,
    ] {
        assert!(Shape::from_json(bad).is_err(), "{bad} accepted");
    }
}

#[test]
fn two_queries_differing_only_in_embedded_document_keys_share_a_shape() {
    let a = shape_of(doc! { "meta": { "user-1": 1 }, "t": { "$eq": { "a": 1 } } });
    let b = shape_of(doc! { "meta": { "user-2": 7, "other": 8 }, "t": { "$eq": { "b": 2 } } });
    assert_eq!(a.hash(), b.hash());
}

#[test]
fn from_json_round_trips_and_is_canonical() {
    let s = shape_of(doc! { "b": { "$in": [1, 2] }, "a": 1, "$or": [{ "z": 1 }, { "y": 1 }] });
    assert_eq!(Shape::from_json(&s.json()).unwrap().hash(), s.hash());
    for non_canonical in [
        r#"{"filter":{"b":"?","a":"?"}}"#,
        r#"{"filter":{"a":{"$in":["?","?"]}}}"#,
        r#"{"filter":{"$or":[{"z":"?"},{"y":"?"}]}}"#,
    ] {
        assert!(Shape::from_json(non_canonical).is_err(), "{non_canonical}");
    }
}

#[test]
fn every_bson_literal_kind_is_masked() {
    let filter = doc! {
        "expr": 1,
        "$expr": { "$gt": ["$spent-SECRET-field", 918273645] },
        "$text": { "$search": "SECRET-search" },
        "$where": "this.pw == 'SECRET-where'",
        "js": Bson::JavaScriptCode("SECRET-js".into()),
        "jsws": Bson::JavaScriptCodeWithScope(JavaScriptCodeWithScope {
            code: "SECRET-jsws".into(), scope: doc! { "SECRET-scope": 5551212 } }),
        "geo": { "$geoWithin": { "$geometry": { "type": "Polygon",
            "coordinates": [[[11.111, 22.222], [33.333, 44.444]]] } } },
        "near": { "$near": { "$geometry": { "type": "Point", "coordinates": [55.555, 66.666] },
            "$maxDistance": 98765 } },
        "docs": { "$in": [{ "k": "SECRET-in" }] },
        "dec": Bson::Decimal128("123456.789".parse::<Decimal128>().unwrap()),
        "ts": Bson::Timestamp(Timestamp { time: 1_234_567_890, increment: 7 }),
        "sym": Bson::Symbol("SECRET-sym".into()),
        "min": Bson::MinKey,
        "max": Bson::MaxKey,
        "undef": Bson::Undefined,
        "null": Bson::Null,
        "re": Bson::RegularExpression(Regex { pattern: "SECRET-re".into(), options: "i".into() }),
        "nested": [[{ "a": "SECRET-nested" }], [{ "b": 424242 }]],
        "$and": [{ "n": [{ "SECRET-and": 1 }] }],
    };
    let json = json_of_filter(filter);
    for leak in [
        "SECRET",
        "918273645",
        "5551212",
        "11.111",
        "22.222",
        "33.333",
        "44.444",
        "55.555",
        "66.666",
        "98765",
        "123456",
        "1234567890",
        "424242",
    ] {
        assert!(!json.contains(leak), "{leak} leaked into {json}");
    }
}

#[test]
fn profile_dispatches_on_the_real_op_and_command_verb() {
    let find = doc! { "find": "c", "filter": { "a": 1 }, "sort": { "b": -1 }, "$db": "d" };
    let want_find = r#"{"filter":{"a":"?"},"sort":{"b":-1}}"#;
    assert_eq!(json_of("query", find.clone()), want_find);
    assert_eq!(json_of("command", find), want_find);
    let want = r#"{"filter":{"a":"?"}}"#;
    assert_eq!(
        json_of(
            "command",
            doc! { "aggregate": "c", "pipeline": [{ "$match": { "a": 1 } }, { "$sort": { "b": 1 } }], "cursor": {} }
        ),
        r#"{"filter":{"a":"?"},"sort":{"b":1}}"#
    );
    assert_eq!(
        json_of("command", doc! { "count": "c", "query": { "a": 1 } }),
        want
    );
    assert_eq!(
        json_of(
            "command",
            doc! { "distinct": "c", "key": "k", "query": { "a": 1 } }
        ),
        want
    );
    assert_eq!(
        json_of(
            "command",
            doc! { "findAndModify": "c", "query": { "a": 1 }, "sort": { "b": 1 }, "update": {} }
        ),
        r#"{"filter":{"a":"?"},"sort":{"b":1}}"#
    );
    assert_eq!(
        json_of(
            "command",
            doc! { "update": "c", "updates": [{ "q": { "a": 1 }, "u": {} }] }
        ),
        want
    );
    assert_eq!(
        json_of(
            "command",
            doc! { "delete": "c", "deletes": [{ "q": { "a": 1 }, "limit": 0 }] }
        ),
        want
    );
    assert_eq!(json_of("update", doc! { "q": { "a": 1 }, "u": {} }), want);
    assert_eq!(
        json_of("remove", doc! { "q": { "a": 1 }, "limit": 0 }),
        want
    );
}

#[test]
fn profile_command_ops_without_a_filter_are_none() {
    assert!(
        from_profile_command("getmore", &doc! { "getMore": 1_i64, "collection": "c" }).is_none()
    );
    assert!(
        from_profile_command("command", &doc! { "getMore": 1_i64, "collection": "c" }).is_none()
    );
    assert!(
        from_profile_command("insert", &doc! { "insert": "c", "documents": [{ "a": 1 }] })
            .is_none()
    );
    assert!(from_profile_command("command", &doc! { "ping": 1 }).is_none());
    assert!(from_profile_command("command", &doc! {}).is_none());
}
