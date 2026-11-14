//! Tests of the review fixes: reserved helper names, the conversion note, Go's negative zero and
//! the range error.

use bson::{Bson, doc};

use super::lossy::NOTE;
use super::*;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn aggregate_of(value: Bson) -> CodeRequest {
    CodeRequest::Aggregate {
        ns: ns(),
        stages: vec![doc! { "$match": { "v": value } }],
    }
}

fn render(language: Language, value: Bson, setup: bool) -> String {
    let options = CodeOptions {
        setup,
        ..CodeOptions::default()
    };
    generate(language, &aggregate_of(value), &options).expect("generates")
}

fn symbol() -> Bson {
    Bson::Symbol("sym".into())
}

fn db_pointer() -> Bson {
    let json = serde_json::json!({
        "$dbPointer": { "$ref": "shop.orders", "$id": { "$oid": "65a1b2c3d4e5f60718293a4b" } }
    });
    Bson::try_from(json).expect("extended json")
}

#[test]
fn names_the_generated_code_relies_on_are_refused() {
    for name in [
        "Map",
        "NumberLong",
        "NumberDecimal",
        "BinData",
        "bytes",
        "float",
        "args",
        "BsonBinarySubType",
        "BsonJavaScriptWithScope",
        "hex2bin",
    ] {
        for field in 0..3 {
            let mut options = CodeOptions::default();
            match field {
                0 => options.client_var = name.into(),
                1 => options.db_var = name.into(),
                _ => options.collection_var = name.into(),
            }
            assert!(options.validated().is_err(), "{name} slot {field}");
        }
    }
}

/// The languages that convert a kind of value, with their comment marker.
fn converting(kind: &str) -> Vec<(Language, &'static str)> {
    match kind {
        "undefined" => vec![
            (Language::Mongosh, "//"),
            (Language::Node, "//"),
            (Language::Python, "#"),
            (Language::Php, "//"),
        ],
        "symbol" => vec![(Language::Python, "#"), (Language::Php, "//")],
        _ => vec![
            (Language::Mongosh, "//"),
            (Language::Node, "//"),
            (Language::Python, "#"),
            (Language::Php, "//"),
            (Language::Rust, "//"),
        ],
    }
}

#[test]
fn a_converted_value_adds_one_fixed_note_at_the_top_of_the_body() {
    let kinds = [
        ("undefined", Bson::Undefined),
        ("symbol", symbol()),
        ("dbPointer", db_pointer()),
    ];
    for (kind, value) in kinds {
        for (language, comment) in converting(kind) {
            for setup in [true, false] {
                let out = render(language, value.clone(), setup);
                let line = format!("{comment} {NOTE}\n");
                assert_eq!(out.matches(NOTE).count(), 1, "{language:?} {kind} {setup}");
                let at = out
                    .find(&line)
                    .unwrap_or_else(|| panic!("{language:?} {kind}"));
                let rest = &out[at + line.len()..];
                let first = rest.lines().next().unwrap_or_default();
                assert!(
                    first.contains("pipeline ="),
                    "{language:?} {kind}: the body follows the note"
                );
                if !setup {
                    assert_eq!(at, 0, "{language:?} {kind}: the note is the first line");
                }
            }
        }
    }
}

#[test]
fn the_note_is_absent_when_nothing_was_converted() {
    for language in Language::ALL {
        for value in [Bson::Int32(1), Bson::String("x".into()), Bson::Null] {
            assert!(
                !render(language, value, true).contains("Note:"),
                "{language:?}"
            );
        }
    }
    // Languages with a literal for the value keep it and say nothing.
    for language in [
        Language::Java,
        Language::CSharp,
        Language::Go,
        Language::Ruby,
    ] {
        for value in [Bson::Undefined, symbol(), db_pointer()] {
            assert!(
                !render(language, value, true).contains("Note:"),
                "{language:?}"
            );
        }
    }
    for value in [Bson::Undefined, symbol()] {
        assert!(!render(Language::Rust, value, true).contains("Note:"));
    }
    assert!(!render(Language::Node, symbol(), true).contains("Note:"));
}

#[test]
fn go_writes_a_negative_zero_that_stays_negative() {
    let out = render(Language::Go, Bson::Double(-0.0), true);
    assert!(out.contains("math.Copysign(0, -1)"), "{out}");
    assert!(out.contains("\t\"math\"\n"), "the math import is added");
    let bare = render(Language::Go, Bson::Double(-0.0), false);
    assert!(bare.contains("math.Copysign(0, -1)"));
    let positive = render(Language::Go, Bson::Double(0.0), true);
    assert!(
        positive.contains("0.0") && !positive.contains("math"),
        "{positive}"
    );
}

#[test]
fn other_languages_keep_their_negative_zero() {
    for language in [Language::Python, Language::Rust, Language::Node] {
        assert!(render(language, Bson::Double(-0.0), true).contains("-0.0"));
    }
}

#[test]
fn an_oversized_skip_is_an_out_of_range_error() {
    let request = CodeRequest::Find {
        ns: ns(),
        query: Query {
            skip: u64::MAX,
            ..Query::default()
        },
    };
    for language in [Language::Java, Language::CSharp, Language::Go] {
        let out = generate(language, &request, &CodeOptions::default());
        assert_eq!(out, Err(CodegenError::OutOfRange("skip")), "{language:?}");
    }
    let limit = CodeRequest::Find {
        ns: ns(),
        query: Query {
            limit: u64::MAX,
            ..Query::default()
        },
    };
    assert_eq!(
        generate(Language::Rust, &limit, &CodeOptions::default()),
        Err(CodegenError::OutOfRange("limit"))
    );
}
