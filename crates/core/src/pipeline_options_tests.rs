use bson::doc;
use serde_json::json;

use super::*;

fn collation(locale: &str) -> CollationOption {
    CollationOption {
        locale: locale.to_string(),
        strength: None,
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    }
}

fn with_locale(locale: &str) -> PipelineOptions {
    PipelineOptions {
        collation: Some(collation(locale)),
        ..PipelineOptions::default()
    }
}

fn with_hint(hint: HintOption) -> PipelineOptions {
    PipelineOptions {
        hint: Some(hint),
        ..PipelineOptions::default()
    }
}

fn with_keys(keys: bson::Document) -> Result<PipelineOptions, OptionsError> {
    with_hint(HintOption::Keys(keys)).validated()
}

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: Some(Strength::Secondary),
            case_level: Some(true),
            case_first: Some(CaseFirst::Upper),
            numeric_ordering: Some(true),
            alternate: Some(Alternate::Shifted),
        }),
        hint: Some(HintOption::Keys(doc! { "a": 1, "b": -1 })),
        allow_disk_use: Some(true),
        max_time_ms: Some(5_000),
        comment: Some("nightly report".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    }
}

fn round_trip(options: &PipelineOptions) -> PipelineOptions {
    let text = serde_json::to_string(options).expect("serializes");
    serde_json::from_str(&text).expect("loads")
}

#[test]
fn the_default_is_empty_and_counts_nothing() {
    let none = PipelineOptions::default();
    assert!(none.is_empty());
    assert_eq!(none.set_count(), 0);
    assert!(!every_option().is_empty());
}

#[test]
fn set_count_counts_each_top_level_option_once() {
    assert_eq!(every_option().set_count(), 6);
    let one = with_locale("en");
    assert_eq!(one.set_count(), 1);
    let two = PipelineOptions {
        comment: Some("x".into()),
        ..one
    };
    assert_eq!(two.set_count(), 2);
    let many_collation_fields = PipelineOptions {
        collation: Some(CollationOption {
            strength: Some(Strength::Primary),
            case_level: Some(false),
            ..collation("en")
        }),
        ..PipelineOptions::default()
    };
    assert_eq!(many_collation_fields.set_count(), 1);
}

#[test]
fn a_valid_locale_passes_and_is_trimmed() {
    let ok = with_locale("  fr  ").validated().expect("valid");
    assert_eq!(ok.collation.expect("collation").locale, "fr");
}

#[test]
fn simple_and_variant_locales_pass() {
    for locale in ["simple", "de@collation=phonebook", "zh_Hant", "en_US", "fr"] {
        assert!(with_locale(locale).validated().is_ok(), "{locale}");
    }
}

#[test]
fn bad_locales_are_refused() {
    let long = "a".repeat(33);
    assert_eq!(with_locale("").validated(), Err(OptionsError::LocaleEmpty));
    assert_eq!(
        with_locale("   ").validated(),
        Err(OptionsError::LocaleEmpty)
    );
    for locale in [
        "en US",
        "../x",
        "e",
        "en_",
        "en@collation=",
        "1en",
        long.as_str(),
    ] {
        assert_eq!(
            with_locale(locale).validated(),
            Err(OptionsError::LocaleInvalid),
            "{locale}"
        );
    }
}

#[test]
fn hint_parse_reads_a_name_or_a_key_pattern() {
    assert_eq!(
        HintOption::parse("  by_name_1 "),
        Ok(HintOption::Name("by_name_1".into()))
    );
    assert_eq!(
        HintOption::parse(r#" {"a": 1, "b": -1} "#),
        Ok(HintOption::Keys(doc! { "a": 1, "b": -1 }))
    );
    assert_eq!(HintOption::parse("   "), Err(OptionsError::HintEmpty));
    let long = "n".repeat(257);
    assert_eq!(HintOption::parse(&long), Err(OptionsError::HintNameTooLong));
    assert!(HintOption::parse(&"n".repeat(256)).is_ok());
}

#[test]
fn hint_keys_must_be_directions_or_index_types() {
    for keys in [
        doc! {},
        doc! { "a": 2 },
        doc! { "a": true },
        doc! { "a": "" },
        doc! { "$natural": 1, "a": 1 },
        doc! { "$natural": "text" },
    ] {
        assert_eq!(
            with_keys(keys.clone()),
            Err(OptionsError::HintKeysInvalid),
            "{keys:?}"
        );
    }
    for keys in [
        doc! { "a": 1, "b": -1 },
        doc! { "t": "text" },
        doc! { "$natural": -1 },
        doc! { "g": "2dsphere", "a": 1_i64 },
        doc! { "a": 1.0 },
    ] {
        assert!(with_keys(keys.clone()).is_ok(), "{keys:?}");
    }
}

#[test]
fn a_hint_that_is_not_json_is_hint_json() {
    assert_eq!(HintOption::parse("{not json"), Err(OptionsError::HintJson));
    assert_eq!(HintOption::parse("{\"a\": }"), Err(OptionsError::HintJson));
}

#[test]
fn a_hint_name_is_trimmed_and_checked_on_validation() {
    let ok = with_hint(HintOption::Name("  idx  ".into()))
        .validated()
        .expect("valid");
    assert_eq!(ok.hint, Some(HintOption::Name("idx".into())));
    assert_eq!(
        with_hint(HintOption::Name(" ".into())).validated(),
        Err(OptionsError::HintEmpty)
    );
    assert_eq!(
        with_hint(HintOption::Name("a\nb".into())).validated(),
        Err(OptionsError::HintEmpty)
    );
}

#[test]
fn max_time_bounds() {
    let at = |ms| {
        PipelineOptions {
            max_time_ms: Some(ms),
            ..PipelineOptions::default()
        }
        .validated()
    };
    assert_eq!(at(0), Err(OptionsError::MaxTimeRange));
    assert_eq!(at(60_001), Err(OptionsError::MaxTimeRange));
    assert!(at(1).is_ok());
    assert!(at(60_000).is_ok());
    assert_eq!(MAX_TIME_MS_CAP, 60_000);
}

#[test]
fn comments_are_bounded_and_printable() {
    let at = |text: &str| {
        PipelineOptions {
            comment: Some(text.to_string()),
            ..PipelineOptions::default()
        }
        .validated()
    };
    assert_eq!(at(""), Err(OptionsError::CommentEmpty));
    assert_eq!(at("  "), Err(OptionsError::CommentEmpty));
    assert_eq!(at(&"c".repeat(257)), Err(OptionsError::CommentTooLong));
    assert_eq!(at("a\u{0}b"), Err(OptionsError::CommentControl));
    assert_eq!(at("a\tb"), Err(OptionsError::CommentControl));
    assert!(at(&"c".repeat(256)).is_ok());
    let trimmed = at("  report  ").expect("valid");
    assert_eq!(trimmed.comment.as_deref(), Some("report"));
}

#[test]
fn serde_uses_mongodb_names() {
    let value = serde_json::to_value(every_option()).expect("serializes");
    assert_eq!(
        value,
        json!({
            "collation": {
                "locale": "en",
                "strength": 2,
                "caseLevel": true,
                "caseFirst": "upper",
                "numericOrdering": true,
                "alternate": "shifted"
            },
            "hint": { "keys": { "a": 1, "b": -1 } },
            "allowDiskUse": true,
            "maxTimeMS": 5000,
            "comment": "nightly report",
            "readPreference": "secondaryPreferred"
        })
    );
    let named = with_hint(HintOption::Name("by_a".into()));
    assert_eq!(
        serde_json::to_value(named).expect("serializes"),
        json!({ "hint": { "name": "by_a" } })
    );
    assert_eq!(
        serde_json::to_value(PipelineOptions::default()).expect("serializes"),
        json!({})
    );
}

#[test]
fn serde_round_trips_every_field() {
    assert_eq!(round_trip(&every_option()), every_option());
    for strength in [
        Strength::Primary,
        Strength::Secondary,
        Strength::Tertiary,
        Strength::Quaternary,
        Strength::Identical,
    ] {
        let o = PipelineOptions {
            collation: Some(CollationOption {
                strength: Some(strength),
                ..collation("en")
            }),
            ..PipelineOptions::default()
        };
        assert_eq!(round_trip(&o), o);
    }
    for mode in [
        ReadPreferenceMode::Primary,
        ReadPreferenceMode::PrimaryPreferred,
        ReadPreferenceMode::Secondary,
        ReadPreferenceMode::SecondaryPreferred,
        ReadPreferenceMode::Nearest,
    ] {
        let o = PipelineOptions {
            read_preference: Some(mode),
            ..PipelineOptions::default()
        };
        assert_eq!(round_trip(&o), o);
    }
    for case_first in [CaseFirst::Upper, CaseFirst::Lower, CaseFirst::Off] {
        let o = PipelineOptions {
            collation: Some(CollationOption {
                case_first: Some(case_first),
                alternate: Some(Alternate::NonIgnorable),
                ..collation("en")
            }),
            ..PipelineOptions::default()
        };
        assert_eq!(round_trip(&o), o);
    }
    let named = with_hint(HintOption::Name("by_a".into()));
    assert_eq!(round_trip(&named), named);
}

#[test]
fn unknown_keys_are_ignored_on_load() {
    let loaded: PipelineOptions = serde_json::from_value(json!({
        "comment": "x",
        "futureThing": [1, 2],
        "collation": { "locale": "en", "maxVariable": "punct" }
    }))
    .expect("loads");
    assert_eq!(loaded.comment.as_deref(), Some("x"));
    assert_eq!(loaded.collation.expect("collation").locale, "en");
}

#[test]
fn an_invalid_known_value_fails_to_load() {
    for bad in [
        json!({ "collation": { "locale": "en", "strength": 9 } }),
        json!({ "readPreference": "tagged" }),
        json!({ "maxTimeMS": 0 }),
        json!({ "maxTimeMS": 60001 }),
        json!({ "collation": { "locale": "en US" } }),
        json!({ "hint": { "keys": { "a": 2 } } }),
        json!({ "comment": "" }),
        json!({ "collation": { "locale": "en", "caseFirst": "sideways" } }),
    ] {
        assert!(
            serde_json::from_value::<PipelineOptions>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn loading_trims_like_validation() {
    let loaded: PipelineOptions =
        serde_json::from_value(json!({ "comment": "  hi  ", "hint": { "name": " i " } }))
            .expect("loads");
    assert_eq!(loaded.comment.as_deref(), Some("hi"));
    assert_eq!(loaded.hint, Some(HintOption::Name("i".into())));
}

#[test]
fn error_variants_are_stable_and_content_free() {
    let all = [
        (OptionsError::LocaleEmpty, "locale_empty"),
        (OptionsError::LocaleInvalid, "locale_invalid"),
        (OptionsError::HintEmpty, "hint_empty"),
        (OptionsError::HintNameTooLong, "hint_name_too_long"),
        (OptionsError::HintKeysInvalid, "hint_keys_invalid"),
        (OptionsError::HintJson, "hint_json"),
        (OptionsError::MaxTimeRange, "max_time_range"),
        (OptionsError::CommentEmpty, "comment_empty"),
        (OptionsError::CommentTooLong, "comment_too_long"),
        (OptionsError::CommentControl, "comment_control"),
    ];
    for (error, variant) in all {
        assert_eq!(error.error_variant(), variant);
    }
}
