use super::*;
use bson::{Bson, DateTime, doc, oid::ObjectId};

fn parsed(text: &str) -> Document {
    parse_literal(text).unwrap_or_else(|| panic!("a literal: {text}"))
}

#[test]
fn bare_and_quoted_keys() {
    assert_eq!(
        parsed(r#"{a: 1, "b": 2, 'c': 3, $gt: 4, _x$: 5, 7: 6}"#),
        doc! { "a": 1, "b": 2, "c": 3, "$gt": 4, "_x$": 5, "7": 6 }
    );
}

#[test]
fn single_and_double_quoted_strings_with_escapes() {
    assert_eq!(
        parsed(r#"{a: 'O\'Brien', b: "say \"hi\"", c: 'tab\tnl\n', d: "\\", e: '\x41é\u{1F600}'}"#),
        doc! { "a": "O'Brien", "b": "say \"hi\"", "c": "tab\tnl\n", "d": "\\", "e": "Aé😀" }
    );
    // A surrogate pair escape is one character; a line continuation adds nothing.
    assert_eq!(parsed(r#"{a: "😀"}"#), doc! { "a": "😀" });
    assert_eq!(parsed("{a: 'x\\\ny'}"), doc! { "a": "xy" });
}

#[test]
fn non_ascii_text_in_keys_strings_and_comments() {
    assert_eq!(
        parsed("{café: 'crème 😀', '日本': \"語\" /* ü 😀 */, // ß\n n: 1}"),
        doc! { "café": "crème 😀", "日本": "語", "n": 1 }
    );
}

#[test]
fn integers_fit_i32_then_i64_and_everything_else_is_f64() {
    let d = parsed(
        "{a: 1, b: -2, c: 3000000000, d: -3000000000, e: 1.5, f: 1e3, g: .5, h: 99999999999999999999}",
    );
    assert_eq!(d.get("a"), Some(&Bson::Int32(1)));
    assert_eq!(d.get("b"), Some(&Bson::Int32(-2)));
    assert_eq!(d.get("c"), Some(&Bson::Int64(3_000_000_000)));
    assert_eq!(d.get("d"), Some(&Bson::Int64(-3_000_000_000)));
    assert_eq!(d.get("e"), Some(&Bson::Double(1.5)));
    assert_eq!(d.get("f"), Some(&Bson::Double(1000.0)));
    assert_eq!(d.get("g"), Some(&Bson::Double(0.5)));
    assert_eq!(d.get("h"), Some(&Bson::Double(1e20)));
}

#[test]
fn booleans_null_nested_objects_and_arrays() {
    assert_eq!(
        parsed(
            "{a: true, b: false, c: null, d: {e: [1, {f: null}, []], g: {}}, $or: [{x: 1}, {y: 'z'}],}"
        ),
        doc! {
            "a": true, "b": false, "c": Bson::Null,
            "d": { "e": [1, { "f": Bson::Null }, []], "g": {} },
            "$or": [{ "x": 1 }, { "y": "z" }],
        }
    );
}

#[test]
fn object_id_and_iso_date() {
    let oid = ObjectId::parse_str("507f1f77bcf86cd799439011").expect("hex");
    let date = DateTime::parse_rfc3339_str("2024-05-01T10:20:30Z").expect("date");
    let midnight = DateTime::parse_rfc3339_str("2024-05-01T00:00:00Z").expect("date");
    assert_eq!(
        parsed(
            r#"{_id: ObjectId("507f1f77bcf86cd799439011"), at: ISODate('2024-05-01T10:20:30Z'), d: ISODate("2024-05-01")}"#
        ),
        doc! { "_id": oid, "at": date, "d": midnight }
    );
    assert_eq!(
        parsed(r#"{at: ISODate( "2024-05-01T10:20:30.000+00:00" )}"#),
        doc! { "at": date }
    );
}

#[test]
fn empty_object_with_surrounding_space_and_comments() {
    assert_eq!(parsed("  /* a */ {} // b\n"), Document::new());
}

#[test]
fn rejected_forms_are_none() {
    let rejected = [
        "",
        "   ",
        "[1, 2]",
        "1",
        "'a'",
        "{a: x}",
        "{a}",
        "{a: f(1)}",
        "{...x}",
        "{a: 1, ...x}",
        "{[k]: 1}",
        "{a: `t`}",
        "{a: /re/}",
        "{a: /re/i}",
        "{a: new Date()}",
        "{a: new ISODate('2024-05-01')}",
        "{a: 1} x",
        "{a: 1}}",
        "{a: 1 2}",
        "{a: }",
        "{: 1}",
        "{a 1}",
        "{a: 1,, b: 2}",
        "{,}",
        "{a: undefined}",
        "{a: NaN}",
        "{a: Infinity}",
        "{a: 0x10}",
        "{a: 1_000}",
        "{a: 5n}",
        "{a: - 1}",
        "{a: --1}",
        "{a: 1.2.3}",
        "{a: 1e}",
        "{a: ObjectId('xyz')}",
        "{a: ObjectId('507f1f77bcf86cd79943901')}",
        "{a: ObjectId()}",
        "{a: ObjectId(\"507f1f77bcf86cd799439011\", 1)}",
        "{a: ISODate('not a date')}",
        "{a: ISODate()}",
        "{a: ISODate(x)}",
        "{a: NumberLong(5)}",
        "{a: 'unterminated}",
        "{a: 'line\nbreak'}",
        "{a: '\\uD83D'}",
        "{a: 1",
        "{a: [1, 2}",
        "{a: 1 /* open",
        "{a: function () {}}",
        "{a: () => 1}",
        "{a: 1 + 2}",
    ];
    for text in rejected {
        assert_eq!(parse_literal(text), None, "should be rejected: {text:?}");
    }
}

fn nested(depth: usize) -> String {
    format!("{}{}", "{a: ".repeat(depth - 1) + "{", "}".repeat(depth))
}

#[test]
fn depth_limit_is_sixty_four() {
    assert!(parse_literal(&nested(MAX_DEPTH)).is_some());
    assert_eq!(parse_literal(&nested(MAX_DEPTH + 1)), None);
    let arrays = format!("{{a: {}{}}}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    assert_eq!(parse_literal(&arrays), None);
}

#[test]
fn huge_nesting_and_long_lines_of_braces_neither_overflow_nor_hang() {
    assert_eq!(parse_literal(&"{".repeat(200_000)), None);
    assert_eq!(parse_literal(&"{a: [".repeat(100_000)), None);
    assert_eq!(parse_literal(&nested(100_000)), None);
}

#[test]
fn a_large_flat_literal_parses() {
    let body: Vec<String> = (0..50_000).map(|n| format!("k{n}: {n}")).collect();
    let doc = parsed(&format!("{{{}}}", body.join(", ")));
    assert_eq!(doc.len(), 50_000);
    assert_eq!(doc.get("k49999"), Some(&Bson::Int32(49_999)));
}

#[test]
fn string_literal_and_blank_helpers() {
    assert_eq!(parse_string(" 'users' "), Some("users".to_string()));
    assert_eq!(parse_string("\"a😀\" /* c */"), Some("a😀".to_string()));
    assert_eq!(parse_string("name"), None);
    assert_eq!(parse_string("'a' + 'b'"), None);
    assert_eq!(parse_string("`a`"), None);
    assert!(is_blank("  /* x */ // y\n "));
    assert!(is_blank(""));
    assert!(!is_blank(" {} "));
    assert!(!is_blank("/* open"));
}
