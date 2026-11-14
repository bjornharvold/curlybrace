//! Tests of the Java, C#, Go, PHP, Ruby and Rust renderers.

use bson::{Bson, doc};

use super::*;

const NEW_SIX: [Language; 6] = [
    Language::Java,
    Language::CSharp,
    Language::Go,
    Language::Php,
    Language::Ruby,
    Language::Rust,
];

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn agg(
    language: Language,
    stages: Vec<Document>,
    options: &CodeOptions,
) -> Result<String, CodegenError> {
    generate(
        language,
        &CodeRequest::Aggregate { ns: ns(), stages },
        options,
    )
}

fn value(language: Language, v: Bson) -> Result<String, CodegenError> {
    agg(
        language,
        vec![doc! { "$match": { "v": v } }],
        &CodeOptions::default(),
    )
}

#[test]
fn an_empty_pipeline_is_an_error_for_the_new_languages() {
    for language in NEW_SIX {
        assert_eq!(
            agg(language, vec![], &CodeOptions::default()),
            Err(CodegenError::EmptyPipeline)
        );
    }
}

#[test]
fn the_new_languages_connect_with_the_placeholder_and_drop_setup_on_request() {
    let stages = vec![doc! { "$match": { "a": 1 } }];
    let off = CodeOptions {
        setup: false,
        ..CodeOptions::default()
    };
    for language in NEW_SIX {
        let on = agg(language, stages.clone(), &CodeOptions::default()).unwrap();
        assert!(on.contains("<your connection string>"), "{language:?}");
        assert!(on.ends_with('\n'));
        let bare = agg(language, stages.clone(), &off).unwrap();
        assert!(!bare.contains("connection string"), "{language:?}");
        assert!(
            bare.contains("ggregate(") || bare.contains("Aggregate<"),
            "{language:?}"
        );
    }
}

#[test]
fn a_write_stage_is_exported_or_dropped_in_the_new_languages() {
    let stages = vec![doc! { "$match": { "a": 1 } }, doc! { "$out": "totals" }];
    let off = CodeOptions {
        write_stage: false,
        ..CodeOptions::default()
    };
    for language in NEW_SIX {
        assert!(
            agg(language, stages.clone(), &CodeOptions::default())
                .unwrap()
                .contains("totals")
        );
        assert!(
            !agg(language, stages.clone(), &off)
                .unwrap()
                .contains("totals")
        );
    }
}

#[test]
fn java_never_leaves_a_unicode_escape_for_the_lexer_to_translate() {
    // Raw `\u000a` in the data must stay inert: the backslash is doubled.
    let out = value(Language::Java, Bson::String("\\u000a\"; evil(); //".into())).unwrap();
    assert!(out.contains("\"\\\\u000a\\\"; evil(); //\""));
    // Controls are octal escapes, never `\u00XX`.
    let ctl = value(Language::Java, Bson::String("a\nb\u{0}c\u{1f}".into())).unwrap();
    assert!(ctl.contains("\"a\\nb\\000c\\037\""));
    assert!(!ctl.contains("\\u00"));
}

#[test]
fn java_arrays_of_one_null_do_not_become_a_null_varargs_array() {
    let out = value(Language::Java, Bson::Array(vec![Bson::Null])).unwrap();
    assert!(out.contains("Arrays.asList((Object) null)"));
}

#[test]
fn java_and_csharp_refuse_a_skip_or_limit_beyond_int() {
    for language in [Language::Java, Language::CSharp] {
        let request = CodeRequest::Find {
            ns: ns(),
            query: Query {
                skip: u64::from(u32::MAX),
                ..Query::default()
            },
        };
        assert_eq!(
            generate(language, &request, &CodeOptions::default()),
            Err(CodegenError::OutOfRange("skip"))
        );
    }
}

#[test]
fn csharp_never_uses_interpolated_strings() {
    let out = value(
        Language::CSharp,
        Bson::String("{0} ${x} #{y} \"q\" \\".into()),
    )
    .unwrap();
    assert!(out.contains("\"{0} ${x} #{y} \\\"q\\\" \\\\\""));
    assert!(!out.contains("$\""));
}

#[test]
fn csharp_timestamps_beyond_int_are_cast_unchecked() {
    let ts = Bson::Timestamp(bson::Timestamp {
        time: 4_000_000_000,
        increment: 1,
    });
    let out = value(Language::CSharp, ts).unwrap();
    assert!(out.contains("new BsonTimestamp(unchecked((int)4000000000L), 1)"));
}

#[test]
fn go_uses_the_v2_driver_and_only_imports_math_when_needed() {
    let plain = value(Language::Go, Bson::Int32(1)).unwrap();
    assert!(plain.contains("go.mongodb.org/mongo-driver/v2/bson"));
    assert!(!plain.contains("\"math\""));
    let nan = value(Language::Go, Bson::Double(f64::NAN)).unwrap();
    assert!(nan.contains("\t\"math\"\n") && nan.contains("math.NaN()"));
}

#[test]
fn go_escapes_use_code_points_not_raw_bytes() {
    let out = value(Language::Go, Bson::String("a\u{85}b\u{1}".into())).unwrap();
    assert!(out.contains("\"a\\u0085b\\u0001\""));
    assert!(!out.contains("\\x"));
}

#[test]
fn go_decimal128_is_spelled_by_its_two_words() {
    let d = Bson::Decimal128("1.5E+3".parse().unwrap());
    let out = value(Language::Go, d).unwrap();
    assert!(out.contains("bson.NewDecimal128(0x3044000000000000, 0x000000000000000f)"));
}

#[test]
fn custom_names_are_used_by_the_new_languages() {
    let options = CodeOptions {
        client_var: "c1".into(),
        db_var: "d1".into(),
        collection_var: "orders_c".into(),
        ..CodeOptions::default()
    };
    for language in NEW_SIX {
        let out = agg(language, vec![doc! { "$match": {} }], &options).unwrap();
        assert!(out.contains("orders_c"), "{language:?}");
        assert!(out.contains("c1") && out.contains("d1"), "{language:?}");
    }
}

#[test]
fn go_refuses_names_that_would_shadow_an_imported_package() {
    for name in ["mongo", "bson", "fmt", "log", "context", "math", "options"] {
        let options = CodeOptions {
            client_var: name.into(),
            ..CodeOptions::default()
        };
        assert_eq!(
            agg(Language::Go, vec![doc! { "$match": {} }], &options),
            Err(CodegenError::InvalidName("client")),
            "{name}"
        );
    }
}
