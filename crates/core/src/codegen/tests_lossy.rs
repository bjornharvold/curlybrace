//! The conversion note names only what was converted, and a UUID binary that is not 16 bytes
//! never reaches a driver constructor that rejects it.

use bson::spec::BinarySubtype;
use bson::{Binary, Bson, doc};

use super::*;

fn render_all(language: Language, values: Vec<Bson>) -> String {
    let mut fields = Document::new();
    for (i, value) in values.into_iter().enumerate() {
        fields.insert(format!("v{i}"), value);
    }
    let request = CodeRequest::Aggregate {
        ns: Namespace::new("shop", "orders"),
        stages: vec![doc! { "$match": fields }],
    };
    generate(language, &request, &CodeOptions::default()).expect("generates")
}

fn db_pointer() -> Bson {
    Bson::try_from(serde_json::json!({
        "$dbPointer": { "$ref": "shop.orders", "$id": { "$oid": "65a1b2c3d4e5f60718293a4b" } }
    }))
    .expect("extended json")
}

fn uuid(bytes: Vec<u8>) -> Bson {
    Bson::Binary(Binary {
        subtype: BinarySubtype::Uuid,
        bytes,
    })
}

fn note_line(out: &str) -> Option<&str> {
    out.lines().find(|line| line.contains("Note:"))
}

#[test]
fn the_note_of_a_dbpointer_in_rust_does_not_name_undefined_or_symbol() {
    let out = render_all(Language::Rust, vec![db_pointer()]);
    let line = note_line(&out).expect("a note");
    assert!(line.contains("DBPointer"), "{line}");
    assert!(
        !line.contains("undefined") && !line.contains("symbol"),
        "{line}"
    );
}

#[test]
fn rust_keeps_undefined_and_symbol_without_a_note() {
    let out = render_all(
        Language::Rust,
        vec![Bson::Undefined, Bson::Symbol("s".into())],
    );
    assert!(note_line(&out).is_none(), "{out}");
}

#[test]
fn the_note_lists_each_converted_kind_once_and_no_other() {
    let out = render_all(
        Language::Python,
        vec![Bson::Undefined, Bson::Undefined, db_pointer()],
    );
    let line = note_line(&out).expect("a note");
    assert!(line.contains("undefined/DBPointer"), "{line}");
    assert!(!line.contains("symbol"), "{line}");
    let node = render_all(Language::Node, vec![Bson::Symbol("s".into()), db_pointer()]);
    let line = note_line(&node).expect("a note");
    assert!(
        line.contains("DBPointer") && !line.contains("symbol"),
        "{line}"
    );
}

#[test]
fn a_short_uuid_becomes_generic_binary_with_a_note_in_csharp_and_php() {
    for (language, subtype_four, generic) in [
        (
            Language::CSharp,
            "BsonBinarySubType.UuidStandard",
            "BsonBinarySubType.Binary",
        ),
        (Language::Php, "), 4)", "), 0)"),
    ] {
        for bytes in [vec![], vec![1, 2, 3, 4, 5], (0..17).collect()] {
            let out = render_all(language, vec![uuid(bytes)]);
            assert!(!out.contains(subtype_four), "{language:?}: {out}");
            assert!(out.contains(generic), "{language:?}: {out}");
            let line = note_line(&out).expect("a note");
            assert!(line.contains("non-16-byte UUID"), "{line}");
        }
    }
}

#[test]
fn a_sixteen_byte_uuid_keeps_its_subtype_and_has_no_note() {
    for language in [Language::CSharp, Language::Php] {
        let out = render_all(language, vec![uuid((0..16).collect())]);
        assert!(note_line(&out).is_none(), "{language:?}: {out}");
    }
    let csharp = render_all(Language::CSharp, vec![uuid((0..16).collect())]);
    assert!(csharp.contains("BsonBinarySubType.UuidStandard"));
    let php = render_all(Language::Php, vec![uuid((0..16).collect())]);
    assert!(php.contains("), 4)"));
}

#[test]
fn other_languages_keep_a_short_uuid_as_subtype_four_without_a_note() {
    for (language, expected) in [
        (Language::Mongosh, "BinData(4, \"AQIDBAU=\")"),
        (Language::Node, ", 4)"),
        (Language::Python, ", 4)"),
        (Language::Java, "new Binary((byte) 0x04, "),
        (Language::Go, "Subtype: 0x04"),
        (Language::Ruby, ":uuid)"),
        (Language::Rust, "bson::spec::BinarySubtype::Uuid"),
    ] {
        let out = render_all(language, vec![uuid(vec![1, 2, 3, 4, 5])]);
        assert!(out.contains(expected), "{language:?}: {out}");
        assert!(note_line(&out).is_none(), "{language:?}: {out}");
    }
}
