//! The aggregate options in the code: what is carried, what is left out, and that a value can
//! never leave its literal.

use bson::{Document, doc};

use super::*;
use crate::pipeline_options::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};

/// Languages whose renderers spell the options (all nine).
const RENDERING: [Language; 9] = Language::ALL;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn read_stages() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "A" } },
        doc! { "$sort": { "total": -1_i32 } },
    ]
}

fn write_stages() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "A" } },
        doc! { "$out": "archive" },
    ]
}

fn request(stages: Vec<Document>, options: PipelineOptions) -> CodeRequest {
    CodeRequest::Aggregate {
        ns: ns(),
        stages,
        options,
    }
}

fn collation() -> CollationOption {
    CollationOption {
        locale: "en".into(),
        strength: Some(Strength::Secondary),
        case_level: Some(false),
        case_first: Some(CaseFirst::Upper),
        numeric_ordering: Some(true),
        alternate: Some(Alternate::Shifted),
    }
}

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(collation()),
        hint: Some(HintOption::Name("status_1_total_-1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        comment: Some("nightly report".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    }
}

fn comment_only(comment: &str) -> PipelineOptions {
    PipelineOptions {
        comment: Some(comment.into()),
        ..PipelineOptions::default()
    }
}

fn code(language: Language, request: &CodeRequest, options: &CodeOptions) -> String {
    generate(language, request, options).expect("generates")
}

fn without_options() -> CodeOptions {
    CodeOptions {
        include_options: false,
        ..CodeOptions::default()
    }
}

/// The text of a golden file written before the options existed.
fn golden(language: Language, name: &str) -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/codegen")
        .join(language.key())
        .join(format!("{name}.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden {}", path.display()))
}

/// The pipeline of the `pipeline` golden files.
fn golden_pipeline() -> Vec<Document> {
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

/// The pipeline of the `write_stage` golden files.
fn golden_write_stage() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "A" } },
        doc! { "$out": { "db": "shop", "coll": "archive" } },
    ]
}

/// With no options set, the code is the golden files of before the options existed, whether the
/// options are included or not (a stray option line would change the text and fail here).
#[test]
fn no_options_render_the_code_of_today() {
    let cases = [
        ("pipeline", golden_pipeline()),
        ("write_stage", golden_write_stage()),
    ];
    for language in Language::ALL {
        for (name, stages) in &cases {
            for setup in [true, false] {
                let file = if setup {
                    name.to_string()
                } else {
                    format!("{name}_no_setup")
                };
                let expected = golden(language, &file);
                let plain = request(stages.clone(), PipelineOptions::default());
                for include_options in [true, false] {
                    let options = CodeOptions {
                        setup,
                        include_options,
                        ..CodeOptions::default()
                    };
                    assert_eq!(
                        code(language, &plain, &options),
                        expected,
                        "{} {file} include_options={include_options}",
                        language.key()
                    );
                }
            }
        }
    }
}

#[test]
fn include_options_off_ignores_every_option() {
    for language in Language::ALL {
        let set = request(read_stages(), every_option());
        let none = request(read_stages(), PipelineOptions::default());
        assert_eq!(
            code(language, &set, &without_options()),
            code(language, &none, &CodeOptions::default()),
            "{}",
            language.key()
        );
    }
}

#[test]
fn options_reach_the_code_when_included() {
    for language in RENDERING {
        let set = request(read_stages(), every_option());
        let none = request(read_stages(), PipelineOptions::default());
        let with = code(language, &set, &CodeOptions::default());
        assert_ne!(with, code(language, &none, &CodeOptions::default()));
        assert!(with.contains("5000"), "{}", language.key());
        assert!(with.contains("nightly report"), "{}", language.key());
        assert!(with.contains("status_1_total_-1"), "{}", language.key());
    }
}

#[test]
fn the_read_preference_is_left_out_with_a_note_when_the_code_writes() {
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Secondary),
        comment: Some("archive".into()),
        ..PipelineOptions::default()
    };
    for language in [Language::Mongosh, Language::Python] {
        let text = code(
            language,
            &request(write_stages(), options.clone()),
            &CodeOptions::default(),
        );
        assert!(
            text.contains("Note: this pipeline writes with $out or $merge, which runs on the primary, so the read preference is left out."),
            "{}",
            language.key()
        );
        assert!(!text.to_lowercase().contains("secondary"), "{text}");
        assert!(text.contains("archive"));
    }
}

#[test]
fn dropping_the_write_stage_keeps_the_read_preference() {
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Secondary),
        ..PipelineOptions::default()
    };
    let drop_write = CodeOptions {
        write_stage: false,
        ..CodeOptions::default()
    };
    for language in RENDERING {
        let text = code(
            language,
            &request(write_stages(), options.clone()),
            &drop_write,
        );
        assert!(
            text.to_lowercase().contains("secondary"),
            "{}",
            language.key()
        );
        assert!(!text.contains("Note:"), "{}", language.key());
    }
}

#[test]
fn code_writes_follows_the_write_stage_option() {
    let writes = request(write_stages(), PipelineOptions::default());
    let reads = request(read_stages(), PipelineOptions::default());
    let drop_write = CodeOptions {
        write_stage: false,
        ..CodeOptions::default()
    };
    assert!(code_writes(&writes, &CodeOptions::default()));
    assert!(!code_writes(&writes, &drop_write));
    assert!(!code_writes(&reads, &CodeOptions::default()));
    let find = CodeRequest::Find {
        ns: ns(),
        query: Query::default(),
    };
    assert!(!code_writes(&find, &CodeOptions::default()));
}

#[test]
fn invalid_options_are_refused_without_their_value() {
    let bad = PipelineOptions {
        max_time_ms: Some(0),
        comment: Some("secret-comment".into()),
        ..PipelineOptions::default()
    };
    for language in Language::ALL {
        let err = generate(
            language,
            &request(read_stages(), bad.clone()),
            &CodeOptions::default(),
        )
        .expect_err("refused");
        assert_eq!(err, CodegenError::InvalidOptions);
        assert_eq!(err.error_variant(), "InvalidOptions");
        assert!(!err.to_string().contains("secret-comment"));
        assert!(!err.to_string().contains('0'));
    }
}

/// Each renderer's own string literal function, the only door for option text.
fn quoted(language: Language, text: &str) -> String {
    use crate::codegen::{
        csharp_syntax, go_syntax, java_syntax, js_syntax, php_syntax, python_syntax, ruby_syntax,
        rust_syntax,
    };
    match language {
        Language::Mongosh | Language::Node => js_syntax::quote(text),
        Language::Python => python_syntax::quote(text),
        Language::Java => java_syntax::quote(text),
        Language::CSharp => csharp_syntax::quote(text),
        Language::Go => go_syntax::quote(text),
        Language::Php => php_syntax::quote(text),
        Language::Ruby => ruby_syntax::quote(text),
        Language::Rust => rust_syntax::quote(text),
    }
}

#[test]
fn an_option_string_cannot_break_out_of_its_literal() {
    for payload in [
        "\"); drop(); //",
        "'; system('x'); //",
        "a\\\"b${y}#{z}%s{0}\u{202e}",
    ] {
        for language in RENDERING {
            let text = code(
                language,
                &request(read_stages(), comment_only(payload)),
                &CodeOptions::default(),
            );
            let literal = quoted(language, payload);
            assert_eq!(
                text.matches(&literal).count(),
                1,
                "{}: {payload}",
                language.key()
            );
            let rest = text.replace(&literal, "");
            for danger in ["drop()", "system(", "\u{202e}"] {
                assert!(!rest.contains(danger), "{}: {danger}", language.key());
            }
        }
    }
}

#[test]
fn new_snippet_names_are_reserved() {
    for name in [
        "ReadPreference",
        "Duration",
        "readpref",
        "Collation",
        "with_options",
    ] {
        let options = CodeOptions {
            collection_var: name.into(),
            ..CodeOptions::default()
        };
        assert_eq!(
            options.validated(),
            Err(CodegenError::InvalidName("collection")),
            "{name}"
        );
    }
}
