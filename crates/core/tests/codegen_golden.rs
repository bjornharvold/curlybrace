//! Golden output of the code generator. The files under `tests/codegen/` are
//! the spec of the generated code; review diffs by eye before accepting them.
//! `UPDATE_GOLDEN=1 cargo test -p curlybrace-core --test codegen_golden`
//! rewrites them.

use std::path::PathBuf;

use bson::oid::ObjectId;
use bson::spec::BinarySubtype;
use bson::{Binary, Bson, DateTime, Document, JavaScriptCodeWithScope, Regex, Timestamp, doc};
use curlybrace_core::codegen::{CodeOptions, CodeRequest, CodegenError, Language, generate};

use curlybrace_core::store::{Namespace, Query};

const LANGUAGES: [Language; 9] = Language::ALL;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn oid(hex: &str) -> ObjectId {
    ObjectId::parse_str(hex).expect("valid object id")
}

fn ext(json: serde_json::Value) -> Bson {
    Bson::try_from(json).expect("valid extended json")
}

fn all_types() -> Document {
    doc! {
        "double": 1.5_f64,
        "wholeDouble": 1.0_f64,
        "nan": f64::NAN,
        "posInf": f64::INFINITY,
        "negInf": f64::NEG_INFINITY,
        "string": "text",
        "int32": 7_i32,
        "int64": 1_i64 << 40,
        "bool": true,
        "null": Bson::Null,
        "undefined": Bson::Undefined,
        "objectId": oid("65a1b2c3d4e5f60718293a4b"),
        "date": DateTime::from_millis(1_709_210_096_789),
        "dateBefore1970": DateTime::from_millis(-86_400_000),
        "dateYear10000": DateTime::from_millis(253_402_300_800_000),
        "decimal": Bson::Decimal128("1.5E+3".parse().expect("decimal128")),
        "binary": Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 1, 2, 255] },
        "uuid": Binary { subtype: BinarySubtype::Uuid, bytes: (0..16).collect() },
        "userBinary": Binary { subtype: BinarySubtype::UserDefined(0x80), bytes: vec![9] },
        "regex": Regex { pattern: "^a.*z$".into(), options: "i".into() },
        "timestamp": Timestamp { time: 1_700_000_000, increment: 3 },
        "minKey": Bson::MinKey,
        "maxKey": Bson::MaxKey,
        "code": Bson::JavaScriptCode("function () { return 1; }".into()),
        "codeWithScope": Bson::JavaScriptCodeWithScope(JavaScriptCodeWithScope {
            code: "x + 1".into(),
            scope: doc! { "x": 41_i32 },
        }),
        "symbol": ext(serde_json::json!({ "$symbol": "sym" })),
        "dbPointer": ext(serde_json::json!({
            "$dbPointer": { "$ref": "shop.orders", "$id": { "$oid": "65a1b2c3d4e5f60718293a4b" } }
        })),
        "emptyDoc": {},
        "emptyArray": [],
        "nested": { "a": { "b": [1_i32, [2_i32, [3_i32]], { "c": "d" }] } },
        "needs quoting": 1_i32,
        "dotted.key": 2_i32,
        "$dollar": 3_i32,
        "__proto__": 4_i32,
        "": 5_i32,
    }
}

fn pipeline_stages() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": { "$in": ["A", "B"] }, "total": { "$gte": 100_i32 } } },
        doc! { "$lookup": {
            "from": "customers",
            "localField": "customerId",
            "foreignField": "_id",
            "as": "customer",
        } },
        doc! { "$group": {
            "_id": "$customer.region",
            "revenue": { "$sum": "$total" },
            "orders": { "$count": {} },
        } },
        doc! { "$facet": {
            "top": [{ "$sort": { "revenue": -1_i32 } }, { "$limit": 5_i32 }],
            "all": [{ "$count": "n" }],
        } },
    ]
}

fn strings() -> Document {
    doc! {
        "double quote": "say \"hi\"",
        "single quote": "it's",
        "backslash": "C:\\temp\\new",
        "newline": "line1\nline2",
        "tab": "a\tb",
        "control": "nul\u{0} bell\u{7} del\u{7f} ls\u{2028}",
        "unicode": "caf\u{e9} \u{1F600} e\u{301}",
        "sigils": "$var ${expr} #{ruby} %s %d {0} {name} `tick` ${`x`}",
        "regexWithSlash": Regex { pattern: "a/b\"c".into(), options: "ix".into() },
    }
}

fn invisible() -> Document {
    doc! {
        "bidi": "user\u{202e}txt.exe \u{2066}x\u{2069}",
        "zeroWidth": "a\u{200b}b\u{200d}c\u{2060}d",
        "bom": "\u{feff}start",
        "c1": "next\u{85}line",
        "softHyphen": "co\u{ad}op",
        "mongolian": "a\u{180e}b",
        "annotation": "x\u{fff9}y\u{fffa}z\u{fffb}",
        "tags": "flag\u{e0001}\u{e0067}\u{e007f}",
        "marks": "ok: \u{1F600} caf\u{e9} e\u{301} \u{4e2d}\u{6587}",
        "k\u{202e}ey": 1_i32,
    }
}

/// Numbers at their edges: a negative zero, the 64-bit extremes and a timestamp past `i32::MAX`.
fn edge_numbers() -> Document {
    doc! {
        "negativeZero": -0.0_f64,
        "int64Min": i64::MIN,
        "int64Max": i64::MAX,
        "int32Min": i32::MIN,
        "int32Max": i32::MAX,
        "timestamp": Timestamp { time: 3_000_000_000, increment: 4_000_000_000 },
    }
}

/// Binaries of every shape the languages treat differently.
fn binaries() -> Document {
    let binary = |subtype: u8, bytes: Vec<u8>| Binary {
        subtype: BinarySubtype::from(subtype),
        bytes,
    };
    doc! {
        "empty": binary(0, vec![]),
        "emptyUuid": binary(4, vec![]),
        "oldUuid": binary(3, (0..16).collect()),
        "md5": binary(5, (0..16).collect()),
        "shortUuid": binary(4, vec![1, 2, 3, 4, 5]),
        "uuid": binary(4, (0..16).collect()),
    }
}

/// Subtype 9, which a language may have no name for (Ruby refuses it).
fn binary_subtype_nine() -> Document {
    doc! { "nine": Binary { subtype: BinarySubtype::from(9), bytes: vec![1, 2, 3] } }
}

/// Invisible and bidi characters inside values that are not plain strings.
fn invisible_values() -> Document {
    doc! {
        "regex": Regex { pattern: "a\u{202e}b\u{200b}c\u{e0001}".into(), options: "i".into() },
        "code": Bson::JavaScriptCode("x\u{202e} + \u{feff}1 // \u{ad}".into()),
        "scoped": Bson::JavaScriptCodeWithScope(JavaScriptCodeWithScope {
            code: "y\u{2066}".into(),
            scope: doc! { "k\u{200b}": "v\u{e007f}" },
        }),
        "symbol": ext(serde_json::json!({ "$symbol": "s\u{202e}y\u{e0001}m" })),
        "dbPointer": ext(serde_json::json!({
            "$dbPointer": { "$ref": "sh\u{202e}op.or\u{200b}ders", "$id": { "$oid": "65a1b2c3d4e5f60718293a4b" } }
        })),
    }
}

/// Integer-like keys in an order a JavaScript object would not keep.
fn integer_keys() -> Document {
    doc! {
        "plain": { "b": 1_i32, "2": 2_i32, "1": 3_i32 },
        "inOrder": { "1": 1_i32, "2": 2_i32, "b": 3_i32 },
        "leadingZero": { "01": 1_i32, "a": 2_i32 },
        "nested": [{ "x": { "z": 1_i32, "10": 2_i32, "9": 3_i32 } }],
    }
}

fn find_query() -> Query {
    Query {
        filter: doc! { "status": "A", "qty": { "$gt": 10_i32 } },
        sort: Some(doc! { "qty": -1_i32, "name": 1_i32 }),
        projection: Some(doc! { "name": 1_i32, "qty": 1_i32, "_id": 0_i32 }),
        skip: 20,
        limit: 10,
    }
}

fn cases() -> Vec<(&'static str, CodeRequest)> {
    let aggregate = |stages: Vec<Document>| CodeRequest::Aggregate { ns: ns(), stages };
    vec![
        ("all_types", aggregate(vec![doc! { "$match": all_types() }])),
        ("pipeline", aggregate(pipeline_stages())),
        (
            "find",
            CodeRequest::Find {
                ns: ns(),
                query: find_query(),
            },
        ),
        ("strings", aggregate(vec![doc! { "$match": strings() }])),
        ("invisible", aggregate(vec![doc! { "$match": invisible() }])),
        (
            "invisible_values",
            aggregate(vec![doc! { "$match": invisible_values() }]),
        ),
        (
            "edge_numbers",
            aggregate(vec![doc! { "$match": edge_numbers() }]),
        ),
        ("binaries", aggregate(vec![doc! { "$match": binaries() }])),
        (
            "binary_subtype_9",
            aggregate(vec![doc! { "$match": binary_subtype_nine() }]),
        ),
        (
            "integer_keys",
            aggregate(vec![doc! { "$match": integer_keys() }]),
        ),
        (
            "write_stage",
            aggregate(vec![
                doc! { "$match": { "status": "A" } },
                doc! { "$out": { "db": "shop", "coll": "archive" } },
            ]),
        ),
    ]
}

fn fixture_path(language: Language, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/codegen")
        .join(language.key())
        .join(format!("{name}.txt"))
}

fn check(language: Language, name: &str, actual: &str) {
    let path = fixture_path(language, name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().expect("has parent")).expect("create dir");
        std::fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {}", path.display()));
    assert_eq!(actual, expected, "golden mismatch: {}", path.display());
}

/// The refusals the fixtures expect: the shell cannot keep the order of integer-like keys, and
/// Ruby has no name for binary subtype 9.
fn refusal_expected(language: Language, case: &str) -> bool {
    matches!(
        (language, case),
        (Language::Mongosh, "integer_keys") | (Language::Ruby, "binary_subtype_9")
    )
}

#[test]
fn generated_code_matches_the_golden_files() {
    for language in LANGUAGES {
        for (case, request) in cases() {
            for setup in [true, false] {
                let options = CodeOptions {
                    setup,
                    ..CodeOptions::default()
                };
                // A refusal is part of the spec too: the golden file names the error.
                let code = generate(language, &request, &options).unwrap_or_else(|e| {
                    assert!(
                        refusal_expected(language, case),
                        "{} {case}: {e}",
                        language.key()
                    );
                    format!("refused: {}\n", e.error_variant())
                });
                let name = if setup {
                    case.to_string()
                } else {
                    format!("{case}_no_setup")
                };
                check(language, &name, &code);
            }
        }
    }
}

#[test]
fn every_language_renders_the_all_types_fixture() {
    let (_, request) = cases()
        .into_iter()
        .find(|(n, _)| *n == "all_types")
        .expect("case");
    assert_eq!(Language::ALL.len(), 9);
    for language in Language::ALL {
        let code = generate(language, &request, &CodeOptions::default());
        assert!(code.is_ok(), "{}", language.key());
        assert!(
            fixture_path(language, "all_types").exists(),
            "{}",
            language.key()
        );
    }
}

/// No generated file may contain a raw invisible or bidi character.
#[test]
fn no_golden_file_contains_a_raw_invisible_character() {
    let bad = |c: char| {
        let n = c as u32;
        (c.is_control() && c != '\n' && c != '\t')
            || (0x200b..=0x200f).contains(&n)
            || (0x2028..=0x202e).contains(&n)
            || (0x2060..=0x2069).contains(&n)
            || (0xfff9..=0xfffb).contains(&n)
            || (0xe0000..=0xe007f).contains(&n)
            || n == 0xad
            || n == 0x61c
            || n == 0x180e
            || n == 0xfeff
    };
    for language in LANGUAGES {
        for (case, request) in cases() {
            let Ok(code) = generate(language, &request, &CodeOptions::default()) else {
                assert!(
                    refusal_expected(language, case),
                    "{} {case}",
                    language.key()
                );
                continue;
            };
            assert!(!code.chars().any(bad), "{} {case}", language.key());
        }
    }
}

#[test]
fn only_the_shell_refuses_integer_like_keys_out_of_order() {
    let (_, request) = cases()
        .into_iter()
        .find(|(n, _)| *n == "integer_keys")
        .expect("case");
    for language in LANGUAGES {
        let out = generate(language, &request, &CodeOptions::default());
        match language {
            Language::Mongosh => assert_eq!(out, Err(CodegenError::KeyOrder)),
            _ => assert!(out.is_ok(), "{}", language.key()),
        }
    }
    let node = generate(Language::Node, &request, &CodeOptions::default()).expect("generates");
    assert!(
        node.contains("new Map(["),
        "Node keeps the order with a Map"
    );
    let php = generate(Language::Php, &request, &CodeOptions::default()).expect("generates");
    assert!(
        php.contains("(object) ["),
        "PHP wraps the document in an object"
    );
}

#[test]
fn the_write_stage_fixture_drops_the_stage_when_the_option_is_off() {
    let (_, request) = cases()
        .into_iter()
        .find(|(n, _)| *n == "write_stage")
        .expect("case");
    let off = CodeOptions {
        write_stage: false,
        ..CodeOptions::default()
    };
    for language in LANGUAGES {
        let code = generate(language, &request, &off).expect("generates");
        assert!(!code.contains("archive"), "{}", language.key());
    }
}
