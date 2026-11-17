use super::*;
use bson::doc;
use curlybrace_core::pipeline::{
    Alternate, CaseFirst, CollationOption, HintOption, OptionsError, PipelineOptions,
    ReadPreferenceMode, Strength,
};

fn typed(form: OptionsForm, edits: Vec<OptionsEdit>) -> OptionsForm {
    edits.into_iter().fold(form, OptionsForm::edited)
}

fn parsed(form: &OptionsForm) -> PipelineOptions {
    form.parsed().expect("valid").clone()
}

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "de@collation=phonebook".into(),
            strength: Some(Strength::Secondary),
            case_level: Some(true),
            case_first: Some(CaseFirst::Upper),
            numeric_ordering: Some(false),
            alternate: Some(Alternate::Shifted),
        }),
        hint: Some(HintOption::Keys(doc! { "status": 1, "created": -1 })),
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        comment: Some("nightly report".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    }
}

#[test]
fn the_default_form_parses_to_the_empty_options() {
    let form = OptionsForm::default();
    assert_eq!(parsed(&form), PipelineOptions::default());
    assert!(!form.is_blocked());
}

#[test]
fn from_options_round_trips_every_field() {
    let options = every_option();
    let form = OptionsForm::from_options(&options);
    assert_eq!(parsed(&form), options);
    let by_name = PipelineOptions {
        hint: Some(HintOption::Name("status_1".into())),
        ..every_option()
    };
    assert_eq!(parsed(&OptionsForm::from_options(&by_name)), by_name);
    assert_eq!(
        parsed(&OptionsForm::from_options(&PipelineOptions::default())),
        PipelineOptions::default()
    );
}

#[test]
fn an_empty_text_field_is_unset_and_blank_text_is_a_problem() {
    let blank = |edit| typed(OptionsForm::default(), vec![edit]);
    let hint = blank(OptionsEdit::HintText("   ".into()));
    assert_eq!(
        hint.problem(OptionField::Hint),
        Some(OptionsError::HintEmpty)
    );
    let comment = blank(OptionsEdit::Comment("  ".into()));
    assert_eq!(
        comment.problem(OptionField::Comment),
        Some(OptionsError::CommentEmpty)
    );
    let unset = typed(
        hint,
        vec![
            OptionsEdit::HintText(String::new()),
            OptionsEdit::Comment(String::new()),
            OptionsEdit::MaxTime(String::new()),
        ],
    );
    assert_eq!(parsed(&unset), PipelineOptions::default());
}

#[test]
fn collation_off_ignores_the_locale_and_its_fields() {
    let form = typed(
        OptionsForm::default(),
        vec![
            OptionsEdit::CollationOn(true),
            OptionsEdit::Locale("en".into()),
            OptionsEdit::Strength(Some(Strength::Primary)),
            OptionsEdit::CollationOn(false),
        ],
    );
    assert_eq!(parsed(&form).collation, None);
    assert_eq!(form.locale, "en", "kept for when it is turned back on");
    let broken = typed(form, vec![OptionsEdit::Locale("en US".into())]);
    assert!(!broken.is_blocked(), "an ignored locale is not checked");
    let back = typed(broken, vec![OptionsEdit::CollationOn(true)]);
    assert_eq!(
        back.problem(OptionField::Locale),
        Some(OptionsError::LocaleInvalid)
    );
}

#[test]
fn a_locale_problem_is_reported_on_the_locale_field() {
    let with = |locale: &str| {
        typed(
            OptionsForm::default(),
            vec![
                OptionsEdit::CollationOn(true),
                OptionsEdit::Locale(locale.into()),
            ],
        )
    };
    assert_eq!(
        with("").problem(OptionField::Locale),
        Some(OptionsError::LocaleEmpty)
    );
    assert_eq!(
        with("en US").problem(OptionField::Locale),
        Some(OptionsError::LocaleInvalid)
    );
    assert_eq!(
        with(&"e".repeat(33)).problem(OptionField::Locale),
        Some(OptionsError::LocaleInvalid)
    );
    assert_eq!(with("en_US").problem(OptionField::Locale), None);
    assert_eq!(
        with("  ").problem(OptionField::Locale),
        Some(OptionsError::LocaleEmpty)
    );
}

#[test]
fn a_hint_text_starting_with_a_brace_is_a_key_pattern() {
    let hint = |text: &str| {
        typed(
            OptionsForm::default(),
            vec![OptionsEdit::HintText(text.into())],
        )
    };
    assert_eq!(
        parsed(&hint(r#"{"a": 1}"#)).hint,
        Some(HintOption::Keys(doc! { "a": 1 }))
    );
    assert_eq!(
        parsed(&hint("a_1")).hint,
        Some(HintOption::Name("a_1".into()))
    );
    assert_eq!(
        hint(r#"{"a": 2}"#).problem(OptionField::Hint),
        Some(OptionsError::HintKeysInvalid)
    );
    assert_eq!(
        hint("{a").problem(OptionField::Hint),
        Some(OptionsError::HintJson)
    );
}

#[test]
fn max_time_takes_digits_from_1_to_60000() {
    let time = |text: &str| {
        typed(
            OptionsForm::default(),
            vec![OptionsEdit::MaxTime(text.into())],
        )
    };
    assert_eq!(parsed(&time("1")).max_time_ms, Some(1));
    assert_eq!(parsed(&time("60000")).max_time_ms, Some(60_000));
    for bad in [
        "0",
        "60001",
        "1.5",
        "-3",
        "ten",
        " 5",
        "99999999999999999999999",
    ] {
        assert_eq!(
            time(bad).problem(OptionField::MaxTime),
            Some(OptionsError::MaxTimeRange),
            "{bad}"
        );
    }
}

#[test]
fn a_comment_with_a_newline_is_comment_control() {
    let form = typed(
        OptionsForm::default(),
        vec![OptionsEdit::Comment("a\nb".into())],
    );
    assert_eq!(
        form.problem(OptionField::Comment),
        Some(OptionsError::CommentControl)
    );
    let long = typed(
        OptionsForm::default(),
        vec![OptionsEdit::Comment("x".repeat(257))],
    );
    assert_eq!(
        long.problem(OptionField::Comment),
        Some(OptionsError::CommentTooLong)
    );
}

#[test]
fn several_fields_report_their_problems_together() {
    let form = typed(
        OptionsForm::default(),
        vec![
            OptionsEdit::MaxTime("0".into()),
            OptionsEdit::Comment("   ".into()),
            OptionsEdit::HintText("{".into()),
        ],
    );
    let problems = form.parsed().expect_err("blocked");
    assert_eq!(problems.len(), 3);
    assert!(form.is_blocked());
    assert!(form.problem(OptionField::Locale).is_none());
}

#[test]
fn hint_picked_replaces_the_text_and_cleared_empties_it() {
    let form = typed(
        OptionsForm::default(),
        vec![
            OptionsEdit::HintText("{\"a\": 1}".into()),
            OptionsEdit::HintPicked("status_1".into()),
        ],
    );
    assert_eq!(form.hint_text, "status_1");
    assert_eq!(
        parsed(&form).hint,
        Some(HintOption::Name("status_1".into()))
    );
    let cleared = form.edited(OptionsEdit::HintCleared);
    assert_eq!(cleared.hint_text, "");
    assert_eq!(parsed(&cleared).hint, None);
}

#[test]
fn discrete_edits_set_their_field() {
    let form = typed(
        OptionsForm::default(),
        vec![
            OptionsEdit::AllowDiskUse(Tri::On),
            OptionsEdit::ReadPreference(Some(ReadPreferenceMode::Nearest)),
            OptionsEdit::CollationOn(true),
            OptionsEdit::Locale("fr".into()),
            OptionsEdit::CaseLevel(Tri::Off),
            OptionsEdit::NumericOrdering(Tri::On),
            OptionsEdit::CaseFirst(Some(CaseFirst::Lower)),
            OptionsEdit::Alternate(Some(Alternate::NonIgnorable)),
        ],
    );
    let options = parsed(&form);
    assert_eq!(options.allow_disk_use, Some(true));
    assert_eq!(options.read_preference, Some(ReadPreferenceMode::Nearest));
    let collation = options.collation.expect("on");
    assert_eq!(collation.case_level, Some(false));
    assert_eq!(collation.numeric_ordering, Some(true));
    assert_eq!(collation.case_first, Some(CaseFirst::Lower));
    assert_eq!(collation.alternate, Some(Alternate::NonIgnorable));
    assert_eq!(Tri::of(Some(true)), Tri::On);
    assert_eq!(Tri::of(None).value(), None);
}

#[test]
fn typing_edits_wait_for_apply_and_the_others_do_not() {
    let typing = [
        OptionsEdit::Locale(String::new()),
        OptionsEdit::HintText(String::new()),
        OptionsEdit::MaxTime(String::new()),
        OptionsEdit::Comment(String::new()),
    ];
    assert!(typing.iter().all(OptionsEdit::is_typing));
    let discrete = [
        OptionsEdit::CollationOn(true),
        OptionsEdit::Strength(None),
        OptionsEdit::CaseLevel(Tri::On),
        OptionsEdit::CaseFirst(None),
        OptionsEdit::NumericOrdering(Tri::On),
        OptionsEdit::Alternate(None),
        OptionsEdit::HintPicked("a".into()),
        OptionsEdit::HintCleared,
        OptionsEdit::AllowDiskUse(Tri::Off),
        OptionsEdit::ReadPreference(None),
    ];
    assert!(discrete.iter().all(|e| !e.is_typing()));
}

const ALL_PROBLEMS: [OptionsError; 10] = [
    OptionsError::LocaleEmpty,
    OptionsError::LocaleInvalid,
    OptionsError::HintEmpty,
    OptionsError::HintNameTooLong,
    OptionsError::HintKeysInvalid,
    OptionsError::HintJson,
    OptionsError::MaxTimeRange,
    OptionsError::CommentEmpty,
    OptionsError::CommentTooLong,
    OptionsError::CommentControl,
];

#[test]
fn every_problem_has_its_own_literal_key() {
    let catalogue: serde_json::Value =
        serde_json::from_str(include_str!("../../locales/en.json")).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for error in ALL_PROBLEMS {
        let key = problem_key(error);
        assert!(seen.insert(key), "{key} is shared");
        let name = key.strip_prefix("aggregate.options.problem.").expect(key);
        assert_eq!(name, error.error_variant());
        assert!(
            catalogue["aggregate"]["options"]["problem"][name].is_string(),
            "{key} is not in en.json"
        );
    }
}

#[test]
fn problem_messages_fill_the_limit_from_the_constant() {
    let i18n = crate::I18n::english();
    let comment = problem_message(&i18n, OptionsError::CommentTooLong);
    assert!(comment.contains("256"), "{comment}");
    let name = problem_message(&i18n, OptionsError::HintNameTooLong);
    assert!(name.contains("256"), "{name}");
    let time = problem_message(&i18n, OptionsError::MaxTimeRange);
    assert!(time.contains("60000"), "{time}");
    for error in ALL_PROBLEMS {
        let text = problem_message(&i18n, error);
        assert!(
            !text.contains('{') && !text.starts_with("aggregate."),
            "{text}"
        );
    }
}

#[test]
fn key_patterns_read_back_as_compact_json() {
    assert_eq!(
        keys_text(&doc! { "status": 1, "created": -1_i64, "body": "text" }),
        r#"{"status":1,"created":-1,"body":"text"}"#
    );
}
