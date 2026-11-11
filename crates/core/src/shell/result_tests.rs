use super::*;
use bson::{Bson, doc};

fn parsed(line: &str) -> ShellResult {
    parse_result(line).unwrap_or_else(|| panic!("payload should parse: {line}"))
}

const MIXED: &str = r#"{"k":"cursor","t":"Cursor","more":false,"v":[{"_id":{"$oid":"650000000000000000000002"},"when":{"$date":{"$numberLong":"1704164645678"}},"big":{"$numberLong":"9007199254740993"},"dec":{"$numberDecimal":"12.345"},"dbl":{"$numberDouble":"1.5"},"int":{"$numberInt":"7"},"uuid":{"$binary":{"base64":"ASNFZ4mrze8BI0VniavN7w==","subType":"04"}},"raw":{"$binary":{"base64":"aGVsbG8=","subType":"00"}},"ts":{"$timestamp":{"t":1700000000,"i":3}},"re":{"$regularExpression":{"pattern":"ab+c","options":"i"}},"nothing":null}]}"#;

#[test]
fn a_cursor_with_every_bson_type_round_trips() {
    let ShellResult::Documents {
        kind,
        docs,
        more,
        total,
    } = parsed(MIXED)
    else {
        panic!("expected documents");
    };
    assert_eq!(
        (kind, more, total, docs.len()),
        (DocKind::Cursor, false, None, 1)
    );
    let d = &docs[0];
    assert_eq!(
        d.get_object_id("_id").unwrap().to_hex(),
        "650000000000000000000002"
    );
    assert_eq!(
        d.get_datetime("when").unwrap().timestamp_millis(),
        1704164645678
    );
    assert_eq!(d.get_i64("big").unwrap(), 9007199254740993);
    assert_eq!(d.get_decimal128("dec").unwrap().to_string(), "12.345");
    assert_eq!(d.get_f64("dbl").unwrap(), 1.5);
    assert_eq!(d.get_i32("int").unwrap(), 7);
    assert_eq!(d.get_binary_generic("raw").unwrap(), b"hello");
    match d.get("uuid").unwrap() {
        Bson::Binary(b) => {
            assert_eq!(b.subtype, bson::spec::BinarySubtype::Uuid);
            assert_eq!(b.bytes.len(), 16);
        }
        other => panic!("uuid parsed as {other:?}"),
    }
    assert_eq!(
        d.get_timestamp("ts").unwrap(),
        bson::Timestamp {
            time: 1700000000,
            increment: 3
        }
    );
    match d.get("re").unwrap() {
        Bson::RegularExpression(r) => {
            assert_eq!((r.pattern.as_str(), r.options.as_str()), ("ab+c", "i"));
        }
        other => panic!("regex parsed as {other:?}"),
    }
    assert_eq!(d.get("nothing"), Some(&Bson::Null));
}

#[test]
fn a_cursor_with_more_is_flagged() {
    let r = parsed(
        r#"{"k":"cursor","t":"AggregationCursor","more":true,"v":[{"n":{"$numberInt":"0"},"s":"doc0"}]}"#,
    );
    let ShellResult::Documents { more, docs, .. } = r else {
        panic!()
    };
    assert!(more);
    assert_eq!(docs, vec![doc! {"n": 0, "s": "doc0"}]);
}

#[test]
fn an_empty_cursor_is_documents_with_no_docs() {
    let r = parsed(r#"{"k":"cursor","t":"Cursor","more":false,"v":[]}"#);
    assert_eq!(
        r,
        ShellResult::Documents {
            kind: DocKind::Cursor,
            docs: vec![],
            more: false,
            total: None
        }
    );
}

#[test]
fn an_array_of_documents_has_a_total() {
    let r = parsed(
        r#"{"k":"array","n":{"$numberInt":"2"},"more":false,"v":[{"a":{"$numberInt":"1"}},{"a":{"$numberInt":"2"},"d":{"$date":{"$numberLong":"0"}}}]}"#,
    );
    let ShellResult::Documents {
        kind,
        docs,
        more,
        total,
    } = r
    else {
        panic!()
    };
    assert_eq!(
        (kind, more, total, docs.len()),
        (DocKind::Array, false, Some(2), 2)
    );
    assert_eq!(docs[0], doc! {"a": 1});
}

#[test]
fn an_empty_array_is_documents_with_no_docs() {
    let r = parsed(r#"{"k":"array","n":{"$numberInt":"0"},"more":false,"v":[]}"#);
    assert_eq!(
        r,
        ShellResult::Documents {
            kind: DocKind::Array,
            docs: vec![],
            more: false,
            total: Some(0)
        }
    );
}

#[test]
fn an_array_with_a_scalar_is_values_not_documents() {
    let r = parsed(
        r#"{"k":"array","n":{"$numberInt":"3"},"more":false,"v":["scratch","wide",{"a":1}]}"#,
    );
    let ShellResult::Values {
        values,
        more,
        total,
    } = r
    else {
        panic!("expected values")
    };
    assert_eq!((more, total, values.len()), (false, 3, 3));
    assert_eq!(values[0], Bson::String("scratch".into()));
    assert!(matches!(values[2], Bson::Document(_)));
}

#[test]
fn objects_keep_their_type_name() {
    let r = parsed(
        r#"{"k":"object","t":"InsertOneResult","v":{"acknowledged":true,"insertedId":{"$oid":"650000000000000000000002"}}}"#,
    );
    let ShellResult::Object { type_name, doc } = r else {
        panic!()
    };
    assert_eq!(type_name.as_deref(), Some("InsertOneResult"));
    assert_eq!(doc.get_bool("acknowledged"), Ok(true));
    let plain = parsed(r#"{"k":"object","t":null,"v":{"ok":{"$numberInt":"1"}}}"#);
    assert_eq!(
        plain,
        ShellResult::Object {
            type_name: None,
            doc: doc! {"ok": 1}
        }
    );
}

#[test]
fn seven_levels_of_nesting_and_long_strings_survive() {
    let long = "x".repeat(300);
    let line = format!(
        r#"{{"k":"object","t":"Document","v":{{"a":{{"b":{{"c":{{"d":{{"e":{{"f":{{"g":"{long}"}}}}}}}}}}}}}}}}"#
    );
    let ShellResult::Object { doc, .. } = parsed(&line) else {
        panic!()
    };
    let mut cur = &doc;
    for key in ["a", "b", "c", "d", "e", "f"] {
        cur = cur.get_document(key).unwrap();
    }
    assert_eq!(cur.get_str("g").unwrap().len(), 300);
}

#[test]
fn scalars_and_undefined() {
    assert_eq!(
        parsed(r#"{"k":"scalar","v":{"$numberInt":"300"},"u":false}"#),
        ShellResult::Scalar {
            value: Bson::Int32(300),
            undefined: false
        }
    );
    assert_eq!(
        parsed(r#"{"k":"scalar","v":"hello","u":false}"#),
        ShellResult::Scalar {
            value: Bson::String("hello".into()),
            undefined: false
        }
    );
    assert_eq!(
        parsed(r#"{"k":"scalar","v":null,"u":true}"#),
        ShellResult::Scalar {
            value: Bson::Null,
            undefined: true
        }
    );
}

#[test]
fn a_scalar_undefined_is_flagged() {
    let ShellResult::Scalar { undefined, .. } = parsed(r#"{"k":"scalar","v":null,"u":true}"#)
    else {
        panic!()
    };
    assert!(undefined);
}

#[test]
fn too_big_and_unsupported_are_parsed() {
    assert_eq!(
        parsed(r#"{"k":"too_big","bytes":{"$numberInt":"6291964"}}"#),
        ShellResult::TooBig { chars: 6291964 }
    );
    assert_eq!(
        parsed(r#"{"k":"too_big","bytes":4194305}"#),
        ShellResult::TooBig { chars: 4194305 }
    );
    assert_eq!(
        parsed(r#"{"k":"unsupported","t":"Database"}"#),
        ShellResult::Unsupported {
            type_name: Some("Database".into())
        }
    );
    assert_eq!(
        parsed(r#"{"k":"unsupported","t":null}"#),
        ShellResult::Unsupported { type_name: None }
    );
}

#[test]
fn the_envelope_numbers_may_be_plain_or_wrapped() {
    for n in [
        "2",
        r#"{"$numberInt":"2"}"#,
        r#"{"$numberLong":"2"}"#,
        r#"{"$numberDouble":"2.0"}"#,
    ] {
        let line = format!(r#"{{"k":"array","n":{n},"more":false,"v":[{{"a":1}},{{"a":2}}]}}"#);
        let ShellResult::Documents { total, .. } = parsed(&line) else {
            panic!()
        };
        assert_eq!(total, Some(2), "n = {n}");
    }
}

#[test]
fn a_bad_envelope_number_is_none() {
    for n in [
        "-1",
        "1.5",
        r#""2""#,
        r#"{"$numberInt":"x"}"#,
        r#"{"$oid":"650000000000000000000002"}"#,
        "null",
    ] {
        let line = format!(r#"{{"k":"array","n":{n},"more":false,"v":[]}}"#);
        assert_eq!(parse_result(&line), None, "n = {n}");
    }
}

#[test]
fn a_huge_n_is_clamped() {
    let line =
        r#"{"k":"array","n":{"$numberLong":"9000000000000000000"},"more":false,"v":[{"a":1}]}"#;
    let ShellResult::Documents {
        docs, more, total, ..
    } = parsed(line)
    else {
        panic!()
    };
    assert_eq!(docs.len(), 1);
    assert!(more, "a total beyond what was received means more");
    assert_eq!(total, Some(9_000_000_000_000_000_000));
    // an n that does not fit a count at all is refused
    let line = r#"{"k":"array","n":{"$numberLong":"99999999999999999999"},"more":true,"v":[]}"#;
    assert_eq!(parse_result(line), None);
}

#[test]
fn more_than_200_elements_are_cut_to_200_with_more_set() {
    let items = vec![r#"{"a":1}"#; 250].join(",");
    let cursor = format!(r#"{{"k":"cursor","t":"Cursor","more":false,"v":[{items}]}}"#);
    let ShellResult::Documents { docs, more, .. } = parsed(&cursor) else {
        panic!()
    };
    assert_eq!((docs.len(), more), (MAX_RESULT_ELEMENTS, true));

    let array = format!(r#"{{"k":"array","n":250,"more":false,"v":[{items}]}}"#);
    let ShellResult::Documents {
        docs, more, total, ..
    } = parsed(&array)
    else {
        panic!()
    };
    assert_eq!(
        (docs.len(), more, total),
        (MAX_RESULT_ELEMENTS, true, Some(250))
    );

    let values = vec!["1"; 250].join(",");
    let line = format!(r#"{{"k":"array","n":250,"more":false,"v":[{values}]}}"#);
    let ShellResult::Values {
        values,
        more,
        total,
    } = parsed(&line)
    else {
        panic!()
    };
    assert_eq!(
        (values.len(), more, total),
        (MAX_RESULT_ELEMENTS, true, 250)
    );
}

#[test]
fn malformed_json_is_none() {
    for line in [
        "",
        "not json",
        r#"{"k":"cursor""#,
        r#"{"k":"cursor","t":"C","more":false,"v":[]} x"#,
        "[]",
        "42",
        "null",
    ] {
        assert_eq!(parse_result(line), None, "line = {line:?}");
    }
}

#[test]
fn an_unknown_kind_is_none() {
    assert_eq!(parse_result(r#"{"k":"table","v":[]}"#), None);
    assert_eq!(parse_result(r#"{"v":[]}"#), None);
    assert_eq!(parse_result(r#"{"k":3}"#), None);
}

#[test]
fn a_cursor_with_a_non_document_is_none() {
    assert_eq!(
        parse_result(r#"{"k":"cursor","t":"Cursor","more":false,"v":[{"a":1},3]}"#),
        None
    );
}

#[test]
fn missing_or_mistyped_fields_are_none() {
    for line in [
        r#"{"k":"cursor","more":false,"v":[]}"#,
        r#"{"k":"cursor","t":"Cursor","v":[]}"#,
        r#"{"k":"cursor","t":"Cursor","more":"no","v":[]}"#,
        r#"{"k":"cursor","t":"Cursor","more":false,"v":{}}"#,
        r#"{"k":"array","more":false,"v":[]}"#,
        r#"{"k":"object","t":null,"v":[1]}"#,
        r#"{"k":"object","t":null,"v":3}"#,
        r#"{"k":"object","t":5,"v":{}}"#,
        r#"{"k":"scalar","v":1}"#,
        r#"{"k":"scalar","u":false}"#,
        r#"{"k":"too_big"}"#,
        r#"{"k":"unsupported"}"#,
    ] {
        assert_eq!(parse_result(line), None, "line = {line}");
    }
}

#[test]
fn deeply_nested_input_is_none_not_a_panic() {
    let n = 10_000;
    let deep = format!("{}{}", "[".repeat(n), "]".repeat(n));
    assert_eq!(parse_result(&deep), None);
    let line = format!(r#"{{"k":"scalar","v":{deep},"u":false}}"#);
    assert_eq!(parse_result(&line), None);
    // just past the 100 level limit, inside serde_json's own limit
    let deep = format!("{}{}", "[".repeat(101), "]".repeat(101));
    let line = format!(r#"{{"k":"scalar","v":{deep},"u":false}}"#);
    assert_eq!(parse_result(&line), None);
    // a modest nesting is fine
    let ok = format!("{}{}", "[".repeat(50), "]".repeat(50));
    let line = format!(r#"{{"k":"scalar","v":{ok},"u":false}}"#);
    assert!(parse_result(&line).is_some());
}

#[test]
fn debug_never_prints_document_contents() {
    let secret = "SECRET-VALUE";
    let lines = [
        format!(r#"{{"k":"cursor","t":"Cursor","more":false,"v":[{{"p":"{secret}"}}]}}"#),
        format!(r#"{{"k":"array","n":1,"more":false,"v":[{{"p":"{secret}"}}]}}"#),
        format!(r#"{{"k":"array","n":2,"more":false,"v":["{secret}",1]}}"#),
        format!(r#"{{"k":"object","t":"Document","v":{{"{secret}":"{secret}"}}}}"#),
        format!(r#"{{"k":"scalar","v":"{secret}","u":false}}"#),
    ];
    for line in lines {
        let result = parsed(&line);
        let shown = format!("{result:?} {result:#?}");
        assert!(!shown.contains(secret), "leaked in {shown}");
    }
}

#[test]
fn a_non_alphabetical_field_order_survives_parsing() {
    // Guards `serde_json`'s `preserve_order` feature (asked for in crates/core/Cargo.toml, and
    // today also by `bson`): without it the fields would come back sorted (a, m, z), and the
    // table's columns with them.
    let payload = r#"{"k":"cursor","t":"Cursor","more":false,"v":[{"z":{"$numberInt":"1"},"a":{"$numberInt":"2"},"m":{"$numberInt":"3"}}]}"#;
    let ShellResult::Documents { docs, .. } = parsed(payload) else {
        panic!("expected documents");
    };
    let keys: Vec<&str> = docs[0].keys().map(String::as_str).collect();
    assert_eq!(keys, ["z", "a", "m"]);
    let object = r#"{"k":"object","t":null,"v":{"z":true,"a":true,"m":true}}"#;
    let ShellResult::Object { doc, .. } = parsed(object) else {
        panic!("expected an object");
    };
    let keys: Vec<&str> = doc.keys().map(String::as_str).collect();
    assert_eq!(keys, ["z", "a", "m"]);
}
