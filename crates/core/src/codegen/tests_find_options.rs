//! The query's options in find code: what is carried, what is left out, and that a value can
//! never leave its literal.

use bson::doc;

use super::tests_options::{golden, quoted};
use super::*;
use crate::pipeline_options::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};

/// Languages whose find renderers spell the options so far.
const RENDERING: [Language; 5] = [
    Language::Mongosh,
    Language::Node,
    Language::Python,
    Language::Php,
    Language::Ruby,
];

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

/// The query of the `find` golden files.
fn golden_query() -> Query {
    Query {
        filter: doc! { "status": "A", "qty": { "$gt": 10_i32 } },
        sort: Some(doc! { "qty": -1_i32, "name": 1_i32 }),
        projection: Some(doc! { "name": 1_i32, "qty": 1_i32, "_id": 0_i32 }),
        skip: 20,
        limit: 10,
        ..Query::default()
    }
}

fn find(options: PipelineOptions) -> CodeRequest {
    CodeRequest::Find {
        ns: ns(),
        query: Query {
            options,
            ..golden_query()
        },
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

/// With no option set, the code is the golden file of before find options existed, whether the
/// options are included or not.
#[test]
fn no_options_render_today_s_find_code() {
    for language in Language::ALL {
        for setup in [true, false] {
            let file = if setup { "find" } else { "find_no_setup" };
            let expected = golden(language, file);
            for include_options in [true, false] {
                let options = CodeOptions {
                    setup,
                    include_options,
                    ..CodeOptions::default()
                };
                assert_eq!(
                    code(language, &find(PipelineOptions::default()), &options),
                    expected,
                    "{} {file} include_options={include_options}",
                    language.key()
                );
            }
        }
    }
}

#[test]
fn include_off_ignores_every_find_option() {
    for language in Language::ALL {
        assert_eq!(
            code(language, &find(every_option()), &without_options()),
            code(
                language,
                &find(PipelineOptions::default()),
                &CodeOptions::default()
            ),
            "{}",
            language.key()
        );
    }
}

#[test]
fn find_code_carries_the_options_unless_include_is_off() {
    for language in RENDERING {
        let with = code(language, &find(every_option()), &CodeOptions::default());
        let plain = code(
            language,
            &find(PipelineOptions::default()),
            &CodeOptions::default(),
        );
        assert_ne!(with, plain, "{}", language.key());
        assert!(with.contains("5000"), "{}", language.key());
        assert!(with.contains("nightly report"), "{}", language.key());
        assert!(with.contains("status_1_total_-1"), "{}", language.key());
        assert!(with.contains("en"), "{}", language.key());
    }
}

#[test]
fn the_read_preference_is_never_left_out_of_find_code() {
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Secondary),
        ..PipelineOptions::default()
    };
    for language in RENDERING {
        let text = code(language, &find(options.clone()), &CodeOptions::default());
        assert!(
            text.to_lowercase().contains("secondary"),
            "{}: {text}",
            language.key()
        );
        assert!(!text.contains("Note:"), "{}", language.key());
    }
}

#[test]
fn invalid_find_options_are_refused_without_their_value() {
    let bad = PipelineOptions {
        max_time_ms: Some(0),
        comment: Some("secret-comment".into()),
        ..PipelineOptions::default()
    };
    for language in Language::ALL {
        let err =
            generate(language, &find(bad.clone()), &CodeOptions::default()).expect_err("refused");
        assert_eq!(err, CodegenError::InvalidOptions);
        assert!(!err.to_string().contains("secret-comment"));
    }
}

#[test]
fn a_find_option_string_cannot_break_out_of_its_literal() {
    for payload in [
        "\"); drop(); //",
        "'; system('x'); //",
        "a\\\"b${y}#{z}%s{0}\u{202e}",
    ] {
        for language in RENDERING {
            let text = code(
                language,
                &find(comment_only(payload)),
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
fn code_writes_is_false_for_find_with_options() {
    assert!(!code_writes(&find(every_option()), &CodeOptions::default()));
}

#[test]
fn a_key_hint_is_spelled_in_the_language_and_never_as_a_name() {
    let options = PipelineOptions {
        hint: Some(HintOption::Keys(doc! { "status": 1_i32, "total": -1_i32 })),
        ..PipelineOptions::default()
    };
    for language in RENDERING {
        let text = code(language, &find(options.clone()), &CodeOptions::default());
        assert!(text.contains("status"), "{}", language.key());
        assert!(text.contains("total"), "{}", language.key());
    }
}
