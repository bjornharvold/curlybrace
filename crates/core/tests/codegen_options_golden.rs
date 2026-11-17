//! Golden output of the code generator for aggregates that carry options. The files
//! `tests/codegen/<language>/options_*.txt` are the spec of that code; review diffs by eye.
//! `UPDATE_GOLDEN=1 cargo test -p curlybrace-core --test codegen_options_golden` rewrites them.
//! (`codegen_golden.rs` keeps the goldens of code without options, which must not change.)

use std::path::PathBuf;

use bson::{Document, doc};
use curlybrace_core::codegen::{CodeOptions, CodeRequest, Language, generate};
use curlybrace_core::pipeline_options::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};
use curlybrace_core::store::Namespace;

/// Every language spells the options.
const LANGUAGES: [Language; 9] = Language::ALL;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn read_stages() -> Vec<Document> {
    vec![
        doc! { "$match": { "status": "A" } },
        doc! { "$sort": { "total": -1_i32 } },
    ]
}

/// Quotes, backslashes, interpolation sigils and a bidi override: every language must escape it.
fn hostile_comment() -> String {
    "nightly \"report\" 'q' \\ $x ${y} #{z} %s {0} \u{202e}end".to_string()
}

fn only_locale(locale: &str) -> CollationOption {
    CollationOption {
        locale: locale.into(),
        strength: None,
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    }
}

fn cases() -> Vec<(&'static str, CodeRequest)> {
    let all = PipelineOptions {
        collation: Some(CollationOption {
            strength: Some(Strength::Secondary),
            case_level: Some(false),
            case_first: Some(CaseFirst::Upper),
            numeric_ordering: Some(true),
            alternate: Some(Alternate::Shifted),
            ..only_locale("en")
        }),
        hint: Some(HintOption::Name("status_1_total_-1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        comment: Some(hostile_comment()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    };
    let hint_keys = PipelineOptions {
        collation: Some(only_locale("fr_CA")),
        hint: Some(HintOption::Keys(doc! { "status": 1_i32, "total": -1_i32 })),
        allow_disk_use: Some(false),
        read_preference: Some(ReadPreferenceMode::Nearest),
        ..PipelineOptions::default()
    };
    let write_stage = PipelineOptions {
        comment: Some("archive".into()),
        read_preference: Some(ReadPreferenceMode::Secondary),
        ..PipelineOptions::default()
    };
    let request = |stages: Vec<Document>, options: PipelineOptions| CodeRequest::Aggregate {
        ns: ns(),
        stages,
        options,
    };
    vec![
        ("options_all", request(read_stages(), all)),
        ("options_hint_keys", request(read_stages(), hint_keys)),
        (
            "options_write_stage",
            request(
                vec![
                    doc! { "$match": { "status": "A" } },
                    doc! { "$out": "archive" },
                ],
                write_stage,
            ),
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

#[test]
fn the_options_goldens_match() {
    for language in LANGUAGES {
        for (case, request) in cases() {
            for setup in [true, false] {
                let options = CodeOptions {
                    setup,
                    ..CodeOptions::default()
                };
                let code = generate(language, &request, &options)
                    .unwrap_or_else(|e| panic!("{} {case}: {e}", language.key()));
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
fn every_language_renders_the_options_all_case() {
    let (_, request) = cases()
        .into_iter()
        .find(|(name, _)| *name == "options_all")
        .expect("case");
    for language in LANGUAGES {
        let code = generate(language, &request, &CodeOptions::default());
        assert!(code.is_ok(), "{}", language.key());
        assert!(
            fixture_path(language, "options_all").exists(),
            "{}",
            language.key()
        );
    }
}

/// No generated file may contain a raw invisible or bidi character (the comment holds U+202E).
#[test]
fn no_options_golden_holds_a_raw_invisible_character() {
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
            let code = generate(language, &request, &CodeOptions::default()).expect("generates");
            assert!(!code.chars().any(bad), "{} {case}", language.key());
        }
    }
}
