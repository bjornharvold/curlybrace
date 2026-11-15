//! Every language refuses a skip or limit it cannot carry, with the same labelled error.

use bson::doc;

use super::*;

fn find(skip: u64, limit: u64) -> CodeRequest {
    CodeRequest::Find {
        ns: Namespace::new("shop", "orders"),
        query: Query {
            skip,
            limit,
            ..Query::default()
        },
    }
}

fn run(language: Language, skip: u64, limit: u64) -> Result<String, CodegenError> {
    generate(language, &find(skip, limit), &CodeOptions::default())
}

const JS: [Language; 2] = [Language::Mongosh, Language::Node];
const I64_LANGUAGES: [Language; 4] = [
    Language::Python,
    Language::Php,
    Language::Ruby,
    Language::Rust,
];
const JS_MAX: u64 = 1 << 53;

#[test]
fn skip_and_limit_above_i64_max_are_out_of_range_everywhere() {
    for language in Language::ALL {
        for big in [i64::MAX as u64 + 1, u64::MAX] {
            assert_eq!(
                run(language, big, 0),
                Err(CodegenError::OutOfRange("skip")),
                "{language:?} skip {big}"
            );
            assert_eq!(
                run(language, 0, big),
                Err(CodegenError::OutOfRange("limit")),
                "{language:?} limit {big}"
            );
        }
    }
}

#[test]
fn the_largest_i64_is_written_by_the_languages_that_can_carry_it() {
    for language in I64_LANGUAGES {
        let out = run(language, i64::MAX as u64, i64::MAX as u64).expect("generates");
        assert!(out.contains("9223372036854775807"), "{language:?}: {out}");
    }
}

#[test]
fn the_shells_refuse_a_number_beyond_javascript_precision() {
    for language in JS {
        assert_eq!(
            run(language, JS_MAX + 1, 0),
            Err(CodegenError::OutOfRange("skip")),
            "{language:?}"
        );
        assert_eq!(
            run(language, 0, JS_MAX + 1),
            Err(CodegenError::OutOfRange("limit")),
            "{language:?}"
        );
        let edge = run(language, JS_MAX, JS_MAX).expect("2^53 is exact");
        assert!(edge.contains("9007199254740992"), "{language:?}: {edge}");
    }
}

#[test]
fn exported_stages_borrow_the_request_instead_of_copying_it() {
    let stages = vec![doc! { "$match": { "a": 1_i32 } }, doc! { "$out": "target" }];
    let kept = exported_stages(&stages, true).expect("kept");
    assert!(std::ptr::eq(kept.as_ptr(), stages.as_ptr()));
    assert_eq!(kept.len(), 2);
    let dropped = exported_stages(&stages, false).expect("dropped");
    assert!(std::ptr::eq(dropped.as_ptr(), stages.as_ptr()));
    assert_eq!(dropped.len(), 1);
    assert_eq!(
        exported_stages(&stages[1..], false),
        Err(CodegenError::EmptyPipeline)
    );
}
