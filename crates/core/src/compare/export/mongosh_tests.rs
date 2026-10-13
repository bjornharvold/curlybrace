use bson::oid::ObjectId;
use bson::spec::BinarySubtype;
use bson::{Binary, Bson, DateTime, Decimal128, Document, Regex, Timestamp, doc};

use super::*;
use crate::compare::diff::PathSegment::{AnyElement, Field, Index};

fn v(b: Bson) -> String {
    render_value(&b).expect("renders")
}

fn field(name: &str) -> PathSegment {
    Field(name.to_string())
}

#[test]
fn doubles_that_read_as_integers_keep_the_double_type() {
    // mongosh stores a bare `1` as Int32, so integral doubles go through Double().
    assert_eq!(v(Bson::Double(1.0)), "Double(1.0)");
    assert_eq!(v(Bson::Double(0.0)), "Double(0.0)");
    assert_eq!(v(Bson::Double(-5.0)), "Double(-5.0)");
    assert_eq!(v(Bson::Double(1e300)), "Double(1e300)");
    assert_eq!(v(Bson::Double(f64::MAX)), "Double(1.7976931348623157e308)");
}

#[test]
fn negative_zero_keeps_its_sign() {
    assert_eq!(v(Bson::Double(-0.0)), "Double(-0.0)");
}

#[test]
fn fractional_doubles_are_bare_round_tripping_literals() {
    assert_eq!(v(Bson::Double(1.5)), "1.5");
    assert_eq!(v(Bson::Double(-0.1)), "-0.1");
    assert_eq!(v(Bson::Double(1e-7)), "1e-7");
    assert_eq!(v(Bson::Double(5e-324)), "5e-324");
    for x in [
        0.1_f64,
        1.0 / 3.0,
        f64::MIN_POSITIVE,
        1234.5678e-3,
        f64::EPSILON,
    ] {
        let text = v(Bson::Double(x));
        assert_eq!(
            text.parse::<f64>().expect("a plain literal").to_bits(),
            x.to_bits()
        );
    }
}

#[test]
fn non_finite_doubles() {
    assert_eq!(v(Bson::Double(f64::NAN)), "NaN");
    assert_eq!(v(Bson::Double(f64::INFINITY)), "Infinity");
    assert_eq!(v(Bson::Double(f64::NEG_INFINITY)), "-Infinity");
}

#[test]
fn integers_use_their_constructors() {
    assert_eq!(v(Bson::Int32(-5)), "NumberInt(-5)");
    assert_eq!(v(Bson::Int32(i32::MIN)), "NumberInt(-2147483648)");
    assert_eq!(v(Bson::Int64(7)), "NumberLong(\"7\")");
    assert_eq!(
        v(Bson::Int64(9_007_199_254_740_993)),
        "NumberLong(\"9007199254740993\")"
    );
    assert_eq!(
        v(Bson::Int64(i64::MIN)),
        "NumberLong(\"-9223372036854775808\")"
    );
}

#[test]
fn decimal128_goes_through_its_string_form() {
    let d: Decimal128 = "1.50".parse().expect("decimal");
    assert_eq!(v(Bson::Decimal128(d)), "NumberDecimal(\"1.50\")");
    let nan: Decimal128 = "NaN".parse().expect("decimal");
    assert_eq!(v(Bson::Decimal128(nan)), "NumberDecimal(\"NaN\")");
}

#[test]
fn strings_booleans_and_null() {
    assert_eq!(v(Bson::String("a\"b\n".into())), "\"a\\\"b\\n\"");
    assert_eq!(v(Bson::Boolean(true)), "true");
    assert_eq!(v(Bson::Boolean(false)), "false");
    assert_eq!(v(Bson::Null), "null");
}

#[test]
fn symbol_keeps_its_type() {
    assert_eq!(v(Bson::Symbol("s\"".into())), "BSONSymbol(\"s\\\"\")");
}

#[test]
fn object_id_and_min_max_keys() {
    let id = ObjectId::parse_str("507f1f77bcf86cd799439011").expect("oid");
    assert_eq!(
        v(Bson::ObjectId(id)),
        "ObjectId(\"507f1f77bcf86cd799439011\")"
    );
    assert_eq!(v(Bson::MinKey), "MinKey()");
    assert_eq!(v(Bson::MaxKey), "MaxKey()");
}

#[test]
fn dates_in_the_iso_range() {
    let d = |ms| v(Bson::DateTime(DateTime::from_millis(ms)));
    assert_eq!(d(0), "ISODate(\"1970-01-01T00:00:00.000Z\")");
    assert_eq!(d(-1), "ISODate(\"1969-12-31T23:59:59.999Z\")");
    assert_eq!(
        d(1_700_000_000_123),
        "ISODate(\"2023-11-14T22:13:20.123Z\")"
    );
    assert_eq!(d(951_782_400_000), "ISODate(\"2000-02-29T00:00:00.000Z\")");
    assert_eq!(
        d(-62_135_596_800_000),
        "ISODate(\"0001-01-01T00:00:00.000Z\")"
    );
    assert_eq!(
        d(-62_167_219_200_000),
        "ISODate(\"0000-01-01T00:00:00.000Z\")"
    );
    assert_eq!(
        d(253_402_300_799_999),
        "ISODate(\"9999-12-31T23:59:59.999Z\")"
    );
}

#[test]
fn dates_before_1970_with_fractions_floor_correctly() {
    let d = |ms| v(Bson::DateTime(DateTime::from_millis(ms)));
    assert_eq!(d(-1_001), "ISODate(\"1969-12-31T23:59:58.999Z\")");
    assert_eq!(d(-86_400_000), "ISODate(\"1969-12-31T00:00:00.000Z\")");
}

#[test]
fn dates_outside_the_iso_range_use_a_millisecond_date() {
    let d = |ms| v(Bson::DateTime(DateTime::from_millis(ms)));
    assert_eq!(d(-62_167_219_200_001), "new Date(-62167219200001)");
    assert_eq!(d(-62_198_755_200_000), "new Date(-62198755200000)");
    assert_eq!(d(253_402_300_800_000), "new Date(253402300800000)");
    assert_eq!(d(8_640_000_000_000_000), "new Date(8640000000000000)");
    assert_eq!(d(-8_640_000_000_000_000), "new Date(-8640000000000000)");
}

#[test]
fn dates_a_javascript_date_cannot_hold_are_unrenderable() {
    for ms in [
        8_640_000_000_000_001,
        -8_640_000_000_000_001,
        i64::MIN,
        i64::MAX,
    ] {
        assert_eq!(
            render_value(&Bson::DateTime(DateTime::from_millis(ms))),
            Err(RenderError::Unrenderable)
        );
    }
}

#[test]
fn timestamps_use_the_object_form() {
    assert_eq!(
        v(Bson::Timestamp(Timestamp {
            time: 5,
            increment: 7
        })),
        "Timestamp({ t: 5, i: 7 })"
    );
    assert_eq!(
        v(Bson::Timestamp(Timestamp {
            time: u32::MAX,
            increment: u32::MAX
        })),
        "Timestamp({ t: 4294967295, i: 4294967295 })"
    );
}

#[test]
fn regular_expressions_use_bson_regexp_so_slashes_and_flags_survive() {
    let re = |pattern: &str, options: &str| {
        v(Bson::RegularExpression(Regex {
            pattern: pattern.into(),
            options: options.into(),
        }))
    };
    assert_eq!(re("a/b", "i"), "BSONRegExp(\"a/b\", \"i\")");
    assert_eq!(re("^x$", "imsx"), "BSONRegExp(\"^x$\", \"imsx\")");
    assert_eq!(re("", ""), "BSONRegExp(\"\", \"\")");
    assert_eq!(re("\"\\n", "l"), "BSONRegExp(\"\\\"\\\\n\", \"l\")");
}

#[test]
fn javascript_code_with_and_without_scope() {
    assert_eq!(
        v(Bson::JavaScriptCode("function() { return \"1\"; }".into())),
        "Code(\"function() { return \\\"1\\\"; }\")"
    );
    let with_scope = Bson::JavaScriptCodeWithScope(bson::JavaScriptCodeWithScope {
        code: "x + y".into(),
        scope: doc! { "x": 1, "y": "two" },
    });
    assert_eq!(
        v(with_scope),
        "Code(\"x + y\", {\"x\": NumberInt(1), \"y\": \"two\"})"
    );
    let empty_scope = Bson::JavaScriptCodeWithScope(bson::JavaScriptCodeWithScope {
        code: "1".into(),
        scope: Document::new(),
    });
    assert_eq!(v(empty_scope), "Code(\"1\", {})");
}

fn bin(subtype: BinarySubtype, bytes: &[u8]) -> String {
    v(Bson::Binary(Binary {
        subtype,
        bytes: bytes.to_vec(),
    }))
}

#[test]
fn binary_generic_and_other_subtypes_use_bindata() {
    assert_eq!(
        bin(BinarySubtype::Generic, &[1, 2, 3]),
        "BinData(0, \"AQID\")"
    );
    assert_eq!(bin(BinarySubtype::Generic, &[]), "BinData(0, \"\")");
    assert_eq!(
        bin(BinarySubtype::UuidOld, &[1, 2, 3]),
        "BinData(3, \"AQID\")"
    );
    assert_eq!(bin(BinarySubtype::Md5, &[1, 2, 3]), "BinData(5, \"AQID\")");
    assert_eq!(
        bin(BinarySubtype::UserDefined(128), &[1, 2, 3]),
        "BinData(128, \"AQID\")"
    );
    assert_eq!(
        bin(BinarySubtype::Reserved(0x7f), &[255]),
        "BinData(127, \"/w==\")"
    );
}

#[test]
fn binary_uuid_subtype_four_with_sixteen_bytes() {
    let bytes: Vec<u8> = (0..16u8).map(|i| i * 0x11).collect();
    assert_eq!(
        bin(BinarySubtype::Uuid, &bytes),
        "UUID(\"00112233-4455-6677-8899-aabbccddeeff\")"
    );
}

#[test]
fn binary_subtype_four_of_another_length_is_bindata() {
    assert_eq!(bin(BinarySubtype::Uuid, &[1, 2, 3]), "BinData(4, \"AQID\")");
    assert!(bin(BinarySubtype::Uuid, &[0; 17]).starts_with("BinData(4, \""));
}

#[test]
fn base64_matches_the_rfc_4648_vectors() {
    for (raw, expected) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(
            bin(BinarySubtype::Generic, raw.as_bytes()),
            format!("BinData(0, \"{expected}\")")
        );
    }
}

#[test]
fn undefined_and_db_pointer_have_no_faithful_shell_form() {
    assert_eq!(
        render_value(&Bson::Undefined),
        Err(RenderError::Unrenderable)
    );
    let pointer = Bson::try_from(serde_json::json!({
        "$dbPointer": { "$ref": "c", "$id": { "$oid": "507f1f77bcf86cd799439011" } }
    }))
    .expect("a db pointer");
    assert!(matches!(pointer, Bson::DbPointer(_)));
    assert_eq!(render_value(&pointer), Err(RenderError::Unrenderable));
}

#[test]
fn nested_values_inside_arrays_and_documents_are_still_checked() {
    assert_eq!(
        render_value(&Bson::Array(vec![Bson::Int32(1), Bson::Undefined])),
        Err(RenderError::Unrenderable)
    );
    assert_eq!(
        render_document(&doc! { "a": { "b": [Bson::Undefined] } }),
        Err(RenderError::Unrenderable)
    );
}

#[test]
fn arrays_and_documents_nest() {
    assert_eq!(v(Bson::Array(vec![])), "[]");
    assert_eq!(v(Bson::Document(Document::new())), "{}");
    assert_eq!(
        v(Bson::Array(vec![
            Bson::Int32(1),
            Bson::String("a".into()),
            Bson::Null
        ])),
        "[NumberInt(1), \"a\", null]"
    );
    assert_eq!(
        render_document(&doc! { "a": 1, "b": { "c": [1.5, { "d": true }] } }).expect("renders"),
        "{\"a\": NumberInt(1), \"b\": {\"c\": [1.5, {\"d\": true}]}}"
    );
}

#[test]
fn field_names_are_always_json_string_keys() {
    let rendered = render_document(&doc! {
        "a b": 1, "$x": 2, "123": 3, "": 4, "q\"uote": 5, "new\nline": 6, "ünï": 7
    })
    .expect("renders");
    assert_eq!(
        rendered,
        "{\"a b\": NumberInt(1), \"$x\": NumberInt(2), \"123\": NumberInt(3), \"\": NumberInt(4), \
         \"q\\\"uote\": NumberInt(5), \"new\\nline\": NumberInt(6), \"ünï\": NumberInt(7)}"
    );
}

#[test]
fn a_proto_key_is_an_own_property_not_a_prototype_change() {
    let rendered = render_document(&doc! { "__proto__": 1, "ok": 2 }).expect("renders");
    assert_eq!(
        rendered,
        "{[\"__proto__\"]: NumberInt(1), \"ok\": NumberInt(2)}"
    );
}

#[test]
fn hostile_string_values_stay_inside_their_literal() {
    let rendered = render_document(&doc! {
        "k\"}); db.dropDatabase(); ({\"": "v\"); db.dropDatabase(); //\u{2028}</script>"
    })
    .expect("renders");
    assert!(!rendered.contains('\n') && !rendered.contains('\u{2028}'));
    assert_eq!(
        rendered,
        "{\"k\\\"}); db.dropDatabase(); ({\\\"\": \"v\\\"); db.dropDatabase(); //\\u2028</script>\"}"
    );
}

#[test]
fn absurdly_deep_nesting_is_refused_not_a_stack_overflow() {
    let mut value = Bson::Int32(1);
    for _ in 0..1_000 {
        value = Bson::Array(vec![value]);
    }
    assert_eq!(render_value(&value), Err(RenderError::Unrenderable));
}

#[test]
fn nesting_at_bson_depth_limits_still_renders() {
    let mut value = Bson::Int32(1);
    for _ in 0..100 {
        value = Bson::Array(vec![value]);
    }
    assert!(render_value(&value).is_ok());
}

#[test]
fn render_path_table() {
    let cases: Vec<(Vec<PathSegment>, Option<&str>)> = vec![
        (vec![field("a")], Some("a")),
        (vec![field("a"), field("b"), field("c")], Some("a.b.c")),
        (vec![field("a"), Index(2), field("b")], Some("a.2.b")),
        (vec![field("a"), Index(0)], Some("a.0")),
        (vec![field("a"), field("_id")], Some("a._id")),
        (vec![field("a"), field("0")], Some("a.0")),
        (vec![field("ünï"), field("a b")], Some("ünï.a b")),
        (vec![], None),
        (vec![Index(0)], None),
        (vec![AnyElement], None),
        (vec![field("_id")], None),
        (vec![field("_id"), field("x")], None),
        (vec![field("")], None),
        (vec![field("a"), field("")], None),
        (vec![field("a.b")], None),
        (vec![field("a"), field("b.c")], None),
        (vec![field("$set")], None),
        (vec![field("a"), field("$x")], None),
        (vec![field("a\0b")], None),
        (vec![field("a"), AnyElement], None),
        (vec![field("a"), AnyElement, field("b")], None),
    ];
    for (segments, expected) in cases {
        match expected {
            Some(text) => assert_eq!(render_path(&segments), Ok(text.to_string()), "{segments:?}"),
            None => assert_eq!(
                render_path(&segments),
                Err(RenderError::BadName),
                "{segments:?}"
            ),
        }
    }
}

#[test]
fn render_path_refuses_exactly_what_the_store_refuses() {
    let names = ["", "a", "_id", "$x", "a.b", "a\0", "0", "é"];
    let mut paths: Vec<Vec<PathSegment>> = vec![vec![], vec![Index(1)], vec![AnyElement]];
    for first in names {
        paths.push(vec![field(first)]);
        for second in names {
            paths.push(vec![field(first), field(second)]);
            paths.push(vec![field(first), Index(3), field(second)]);
        }
        paths.push(vec![field(first), AnyElement]);
        paths.push(vec![field(first), Index(0)]);
    }
    for path in paths {
        let ours = render_path(&path);
        let store = crate::mongo_write_fields::render_path(&path);
        assert_eq!(ours.is_ok(), store.is_ok(), "{path:?}");
        if let (Ok(a), Ok(b)) = (ours, store) {
            assert_eq!(a, b);
        }
    }
}

#[test]
fn shell_op_mirrors_the_store_write_op() {
    use crate::store::WriteOp;
    let id = Bson::Int32(1);
    let doc = doc! { "_id": 1 };
    assert_eq!(
        ShellOp::from(WriteOp::Insert { doc: doc.clone() }),
        ShellOp::Insert { doc: doc.clone() }
    );
    assert_eq!(
        ShellOp::from(WriteOp::Replace {
            id: id.clone(),
            doc: doc.clone()
        }),
        ShellOp::Replace {
            id: id.clone(),
            doc
        }
    );
    assert_eq!(
        ShellOp::from(WriteOp::Delete { id: id.clone() }),
        ShellOp::Delete { id: id.clone() }
    );
    let set = vec![(vec![field("a")], Bson::Int32(2))];
    let unset = vec![vec![field("b")]];
    assert_eq!(
        ShellOp::from(WriteOp::UpdateFields {
            id: id.clone(),
            set: set.clone(),
            unset: unset.clone()
        }),
        ShellOp::UpdateFields { id, set, unset }
    );
}
