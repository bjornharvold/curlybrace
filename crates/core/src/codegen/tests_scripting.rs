//! Tests of the PHP, Ruby and Rust renderers.

use bson::{Bson, doc};

use super::*;

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
fn php_and_ruby_single_quote_so_nothing_interpolates() {
    let text = "$var ${x} {$y} #{z} #$w it's \\ \"";
    let expected = "'$var ${x} {$y} #{z} #$w it\\'s \\\\ \"'";
    for language in [Language::Php, Language::Ruby] {
        let out = value(language, Bson::String(text.into())).unwrap();
        assert!(out.contains(expected), "{language:?}");
    }
}

#[test]
fn php_and_ruby_double_quote_only_with_every_sigil_escaped() {
    let text = "$var ${x} {$y} #{z} #$w \"q\" \u{202e}";
    let php = value(Language::Php, Bson::String(text.into())).unwrap();
    assert!(php.contains("\"\\$var \\${x} {\\$y} #{z} #\\$w \\\"q\\\" \\u{202e}\""));
    let ruby = value(Language::Ruby, Bson::String(text.into())).unwrap();
    assert!(ruby.contains("\"$var ${x} {$y} \\#{z} \\#$w \\\"q\\\" \\u{202e}\""));
}

#[test]
fn php_wraps_documents_php_would_misread_as_lists() {
    let out = value(
        Language::Php,
        Bson::Document(doc! { "0": 1_i32, "1": 2_i32 }),
    )
    .unwrap();
    assert!(out.contains("(object) ['0' => 1, '1' => 2]"));
    let plain = value(Language::Php, Bson::Document(doc! { "01": 1_i32 })).unwrap();
    assert!(!plain.contains("(object)"));
}

#[test]
fn php_integers_and_dates_are_spelled_exactly() {
    let int64 = value(Language::Php, Bson::Int64(5)).unwrap();
    assert!(int64.contains("new MongoDB\\BSON\\Int64('5')"));
    let min = value(
        Language::Php,
        Bson::DateTime(bson::DateTime::from_millis(i64::MIN)),
    )
    .unwrap();
    assert!(min.contains("UTCDateTime('-9223372036854775808')"));
}

#[test]
fn php_refuses_superglobal_names() {
    let options = CodeOptions {
        db_var: "_SERVER".into(),
        ..CodeOptions::default()
    };
    assert_eq!(
        agg(Language::Php, vec![doc! { "$match": {} }], &options),
        Err(CodegenError::InvalidName("db"))
    );
}

#[test]
fn ruby_refuses_constant_style_names_and_accepts_the_rest() {
    for name in ["Client", "Mongo", "A"] {
        let options = CodeOptions {
            client_var: name.into(),
            ..CodeOptions::default()
        };
        assert_eq!(
            agg(Language::Ruby, vec![doc! { "$match": {} }], &options),
            Err(CodegenError::InvalidName("client")),
            "{name}"
        );
    }
    let ok = CodeOptions {
        client_var: "_c".into(),
        db_var: "d_1".into(),
        collection_var: "orders".into(),
        ..CodeOptions::default()
    };
    let out = agg(Language::Ruby, vec![doc! { "$match": {} }], &ok).unwrap();
    assert!(out.contains("_c = Mongo::Client.new"));
}

#[test]
fn ruby_times_and_binary_subtypes() {
    let before = value(
        Language::Ruby,
        Bson::DateTime(bson::DateTime::from_millis(-1)),
    )
    .unwrap();
    assert!(before.contains("Time.at(-1, 999, :millisecond).utc"));
    let odd = Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::UserDefined(0x81),
        bytes: vec![1],
    });
    assert_eq!(
        value(Language::Ruby, odd.clone()),
        Err(CodegenError::UnsupportedValue("binary"))
    );
    assert!(value(Language::Php, odd).is_ok());
    let int64 = value(Language::Ruby, Bson::Int64(5)).unwrap();
    assert!(int64.contains("BSON::Int64.new(5)"));
}

#[test]
fn rust_imports_the_bson_module_only_when_a_path_needs_it() {
    let plain = value(Language::Rust, Bson::Int32(1)).unwrap();
    assert!(plain.contains("use mongodb::bson::{Document, doc};"));
    let id = bson::oid::ObjectId::parse_str("65a1b2c3d4e5f60718293a4b").unwrap();
    let with = value(Language::Rust, Bson::ObjectId(id)).unwrap();
    assert!(with.contains("use mongodb::bson::{self, Document, doc};"));
}

#[test]
fn rust_wide_documents_raise_the_macro_recursion_limit() {
    // `doc!` recurses once or more per entry; rustc's default of 128 fails
    // from about 45 entries.
    let wide = |n: usize| {
        let mut doc = Document::new();
        for i in 0..n {
            doc.insert(format!("f{i}"), Bson::Int32(1));
        }
        Bson::Document(doc)
    };
    let small = value(Language::Rust, wide(10)).unwrap();
    assert!(!small.contains("recursion_limit"), "{small}");
    let big = value(Language::Rust, wide(60)).unwrap();
    assert!(big.starts_with("#![recursion_limit = \""), "{big}");
    // The attribute is only legal at the crate root: a fragment gets a note.
    let bare = CodeOptions {
        setup: false,
        ..CodeOptions::default()
    };
    let request = CodeRequest::Aggregate {
        ns: ns(),
        stages: vec![doc! { "$match": { "v": wide(60) } }],
    };
    let fragment = generate(Language::Rust, &request, &bare).unwrap();
    assert!(fragment.starts_with("// "), "{fragment}");
    assert!(fragment.contains("#![recursion_limit = \""), "{fragment}");
    let narrow = CodeRequest::Aggregate {
        ns: ns(),
        stages: vec![doc! { "$match": { "v": 1_i32 } }],
    };
    let fragment = generate(Language::Rust, &narrow, &bare).unwrap();
    assert!(!fragment.contains("recursion_limit"), "{fragment}");
}

#[test]
fn rust_strings_are_plain_literals_that_never_format() {
    let out = value(
        Language::Rust,
        Bson::String("{0} {name} \"); evil(); //\n".into()),
    )
    .unwrap();
    assert!(out.contains("\"{0} {name} \\\"); evil(); //\\n\""));
    assert!(!out.contains("format!"));
}

#[test]
fn rust_numbers_keep_their_width() {
    let long = value(Language::Rust, Bson::Int64(i64::MIN)).unwrap();
    assert!(long.contains("-9223372036854775808_i64"));
    assert!(
        value(Language::Rust, Bson::Int32(3))
            .unwrap()
            .contains("3_i32")
    );
}

#[test]
fn rust_find_without_options_is_a_bare_find() {
    let request = CodeRequest::Find {
        ns: ns(),
        query: Query {
            limit: 0,
            ..Query::default()
        },
    };
    let out = generate(Language::Rust, &request, &CodeOptions::default()).unwrap();
    assert!(out.contains(".find(filter)\n        .await?;"));
    assert!(!out.contains(".skip") && !out.contains(".limit"));
}
