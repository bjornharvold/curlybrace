use std::time::Duration;

use bson::{Bson, doc};
use mongodb::error::{CommandError, Error, ErrorKind};
use mongodb::options::{
    CollationAlternate, CollationCaseFirst, CollationStrength, Hint, ReadPreference,
    SelectionCriteria,
};

use super::*;
use crate::explain::ExplainVerbosity;
use crate::mongo_pipeline::aggregate_explain_command;
use crate::pipeline::{
    Alternate, CaseFirst, CollationOption, HintOption, Operation, PipelineOptions,
    ReadPreferenceMode, Strength,
};
use crate::store::Namespace;

fn command_error(code: i32, message: &str) -> Error {
    let cmd: CommandError = bson::from_document(doc! {
        "code": code, "codeName": "BadValue", "errmsg": message
    })
    .expect("CommandError deserialises");
    ErrorKind::Command(cmd).into()
}

fn collation() -> CollationOption {
    CollationOption {
        locale: "de@collation=phonebook".into(),
        strength: Some(Strength::Secondary),
        case_level: Some(true),
        case_first: Some(CaseFirst::Upper),
        numeric_ordering: Some(true),
        alternate: Some(Alternate::Shifted),
    }
}

fn everything() -> PipelineOptions {
    PipelineOptions {
        collation: Some(collation()),
        hint: Some(HintOption::Name("by_a".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(5_000),
        comment: Some("nightly".into()),
        read_preference: Some(ReadPreferenceMode::Secondary),
    }
}

fn applied(options: PipelineOptions, op: Operation) -> AppliedOptions {
    options.applied(op)
}

fn mode_of(criteria: Option<SelectionCriteria>) -> Option<ReadPreference> {
    match criteria {
        Some(SelectionCriteria::ReadPreference(mode)) => Some(mode),
        _ => None,
    }
}

fn mode_name(mode: &ReadPreference) -> &'static str {
    match mode {
        ReadPreference::Primary => "primary",
        ReadPreference::PrimaryPreferred { .. } => "primaryPreferred",
        ReadPreference::Secondary { .. } => "secondary",
        ReadPreference::SecondaryPreferred { .. } => "secondaryPreferred",
        ReadPreference::Nearest { .. } => "nearest",
        _ => "other",
    }
}

#[test]
fn read_options_never_allow_disk_use() {
    // Forced here even when the applied value says otherwise (defence in depth).
    let forced = AppliedOptions {
        allow_disk_use: Some(true),
        ..applied(everything(), Operation::Preview)
    };
    assert_eq!(read_options(&forced).allow_disk_use, Some(false));
    assert_eq!(
        read_options(&AppliedOptions::bounded(
            Operation::Count,
            Duration::from_secs(1)
        ))
        .allow_disk_use,
        Some(false)
    );
}

#[test]
fn read_options_carry_time_collation_hint_comment_and_read_preference() {
    let o = read_options(&applied(everything(), Operation::Preview));
    assert_eq!(o.max_time, Some(Duration::from_millis(5_000)));
    assert_eq!(
        o.collation.expect("collation").locale,
        "de@collation=phonebook"
    );
    assert!(matches!(o.hint, Some(Hint::Name(ref n)) if n == "by_a"));
    assert_eq!(o.comment, Some(Bson::String("nightly".into())));
    assert!(matches!(
        mode_of(o.selection_criteria),
        Some(ReadPreference::Secondary { .. })
    ));
}

#[test]
fn read_options_for_the_default_send_only_the_bounds() {
    let o = read_options(&applied(PipelineOptions::default(), Operation::Preview));
    assert_eq!(o.max_time, Some(Duration::from_secs(10)));
    assert_eq!(o.allow_disk_use, Some(false));
    assert!(o.collation.is_none() && o.hint.is_none() && o.comment.is_none());
    assert!(o.selection_criteria.is_none());
}

#[test]
fn write_options_always_select_the_primary() {
    for read_preference in [None, Some(ReadPreferenceMode::Secondary)] {
        let applied = AppliedOptions {
            read_preference,
            ..applied(everything(), Operation::Run)
        };
        assert!(matches!(
            mode_of(write_options(&applied).selection_criteria),
            Some(ReadPreference::Primary)
        ));
    }
}

#[test]
fn write_options_carry_disk_use_as_applied() {
    let on = write_options(&applied(everything(), Operation::Run));
    assert_eq!(on.allow_disk_use, Some(true));
    assert_eq!(on.max_time, Some(Duration::from_millis(5_000)));
    assert!(on.hint.is_some() && on.collation.is_some());
    let unset = write_options(&applied(PipelineOptions::default(), Operation::Run));
    assert_eq!(unset.allow_disk_use, None);
    assert_eq!(unset.max_time, Some(Duration::from_secs(60)));
}

#[test]
fn driver_collation_maps_every_field() {
    let c = driver_collation(&collation());
    assert_eq!(c.locale, "de@collation=phonebook");
    assert!(matches!(c.strength, Some(CollationStrength::Secondary)));
    assert_eq!(c.case_level, Some(true));
    assert!(matches!(c.case_first, Some(CollationCaseFirst::Upper)));
    assert_eq!(c.numeric_ordering, Some(true));
    assert!(matches!(c.alternate, Some(CollationAlternate::Shifted)));
    let bare = driver_collation(&CollationOption {
        locale: "en".into(),
        strength: None,
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    });
    assert!(bare.strength.is_none() && bare.case_first.is_none() && bare.alternate.is_none());
    for (ours, theirs) in [
        (Strength::Primary, 1_u32),
        (Strength::Tertiary, 3),
        (Strength::Quaternary, 4),
        (Strength::Identical, 5),
    ] {
        let c = driver_collation(&CollationOption {
            strength: Some(ours),
            ..collation()
        });
        assert_eq!(c.strength.map(u32::from), Some(theirs));
    }
}

#[test]
fn driver_hint_maps_name_and_keys() {
    assert!(matches!(
        driver_hint(&HintOption::Name("i".into())),
        Hint::Name(n) if n == "i"
    ));
    assert!(matches!(
        driver_hint(&HintOption::Keys(doc! { "a": 1 })),
        Hint::Keys(k) if k == doc! { "a": 1 }
    ));
}

#[test]
fn every_read_preference_mode_maps() {
    let cases = [
        (ReadPreferenceMode::Primary, "primary"),
        (ReadPreferenceMode::PrimaryPreferred, "primaryPreferred"),
        (ReadPreferenceMode::Secondary, "secondary"),
        (ReadPreferenceMode::SecondaryPreferred, "secondaryPreferred"),
        (ReadPreferenceMode::Nearest, "nearest"),
    ];
    for (mode, name) in cases {
        let applied = AppliedOptions {
            read_preference: Some(mode),
            ..AppliedOptions::bounded(Operation::Preview, Duration::from_secs(1))
        };
        let read = mode_of(read_criteria(&applied)).expect("a read preference");
        assert_eq!(mode_name(&read), name);
    }
}

#[test]
fn explain_fields_are_empty_for_the_default() {
    for op in [Operation::Explain, Operation::Run] {
        assert!(explain_fields(&applied(PipelineOptions::default(), op)).is_empty());
    }
}

#[test]
fn explain_command_carries_the_options_inside_aggregate() {
    let ns = Namespace::new("shop", "orders");
    let command = aggregate_explain_command(
        &ns,
        vec![doc! { "$match": { "a": 1 } }],
        ExplainVerbosity::ExecutionStats,
        &applied(everything(), Operation::Explain),
    );
    let inner = command.get_document("explain").expect("inner");
    assert_eq!(inner.get_str("aggregate"), Ok("orders"));
    assert_eq!(inner.get_str("comment"), Ok("nightly"));
    assert_eq!(inner.get_str("hint"), Ok("by_a"));
    assert_eq!(inner.get_bool("allowDiskUse"), Ok(true));
    let c = inner.get_document("collation").expect("collation");
    assert_eq!(c.get_str("locale"), Ok("de@collation=phonebook"));
    assert_eq!(c.get_i32("strength"), Ok(2));
    assert_eq!(c.get_str("caseFirst"), Ok("upper"));
    assert_eq!(c.get_str("alternate"), Ok("shifted"));
    assert_eq!(c.get_bool("numericOrdering"), Ok(true));
    // The limit stays outside, like the verbosity, and is the lowered one.
    assert_eq!(command.get_i64("maxTimeMS"), Ok(5_000));
    assert!(!inner.contains_key("maxTimeMS"));
    let keyed = aggregate_explain_command(
        &ns,
        vec![],
        ExplainVerbosity::QueryPlanner,
        &applied(
            PipelineOptions {
                hint: Some(HintOption::Keys(doc! { "a": -1 })),
                ..PipelineOptions::default()
            },
            Operation::Explain,
        ),
    );
    assert_eq!(
        keyed
            .get_document("explain")
            .and_then(|e| e.get_document("hint")),
        Ok(&doc! { "a": -1 })
    );
}

fn options_with(hint: bool, collation: bool) -> AppliedOptions {
    applied(
        PipelineOptions {
            hint: hint.then(|| HintOption::Name("missing".into())),
            collation: collation.then(collation_option),
            ..PipelineOptions::default()
        },
        Operation::Preview,
    )
}

fn collation_option() -> CollationOption {
    collation()
}

#[test]
fn classify_with_maps_a_missing_hint_only_when_a_hint_was_sent() {
    let e = command_error(
        2,
        "error: Hint provided does not correspond to an existing index",
    );
    assert_eq!(
        classify_with(&e, &options_with(true, false)),
        CoreError::HintNotFound
    );
    assert!(matches!(
        classify_with(&e, &options_with(false, false)),
        CoreError::Database(_)
    ));
    let other = command_error(2, "something unrelated");
    assert!(matches!(
        classify_with(&other, &options_with(true, false)),
        CoreError::Database(_)
    ));
    let wrong_code = command_error(13, "hint");
    assert_eq!(
        classify_with(&wrong_code, &options_with(true, false)),
        CoreError::Unauthorized
    );
}

#[test]
fn classify_with_maps_a_refused_collation_only_when_one_was_sent() {
    for text in [
        "unknown collation: xx",
        "Field 'locale' is invalid in: { locale: \"xx\" }",
    ] {
        let e = command_error(2, text);
        assert_eq!(
            classify_with(&e, &options_with(false, true)),
            CoreError::CollationRejected
        );
        assert!(matches!(
            classify_with(&e, &options_with(false, false)),
            CoreError::Database(_)
        ));
    }
}

#[test]
fn classify_with_keeps_the_time_limit_and_other_codes() {
    let timeout = command_error(50, "operation exceeded time limit");
    assert_eq!(
        classify_with(&timeout, &options_with(true, true)),
        CoreError::Database("timeout".into())
    );
}

async fn selection_failure() -> Error {
    let options = mongodb::options::ClientOptions::parse(
        "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=300&connectTimeoutMS=300",
    )
    .await
    .expect("uri parses");
    mongodb::Client::with_options(options)
        .expect("client")
        .database("admin")
        .run_command(doc! { "ping": 1 })
        .await
        .expect_err("nothing listens on port 1")
}

#[tokio::test]
async fn classify_with_maps_server_selection_only_for_a_non_primary_read_preference() {
    let e = selection_failure().await;
    assert!(matches!(e.kind.as_ref(), ErrorKind::ServerSelection { .. }));
    let with = |mode: Option<ReadPreferenceMode>| AppliedOptions {
        read_preference: mode,
        ..AppliedOptions::bounded(Operation::Preview, Duration::from_secs(1))
    };
    for mode in [
        ReadPreferenceMode::Secondary,
        ReadPreferenceMode::SecondaryPreferred,
        ReadPreferenceMode::PrimaryPreferred,
        ReadPreferenceMode::Nearest,
    ] {
        assert_eq!(
            classify_with(&e, &with(Some(mode))),
            CoreError::ReadPreferenceUnavailable
        );
    }
    assert_eq!(classify_with(&e, &with(None)), CoreError::Unreachable);
    assert_eq!(
        classify_with(&e, &with(Some(ReadPreferenceMode::Primary))),
        CoreError::Unreachable
    );
}

#[test]
fn classified_errors_hold_no_server_text() {
    let e = command_error(2, "hint zebra-secret collation locale");
    for options in [options_with(true, true), options_with(false, true)] {
        let classified = classify_with(&e, &options);
        assert!(!format!("{classified:?} {classified}").contains("zebra-secret"));
    }
}

#[test]
fn is_primary_is_only_the_primary_mode() {
    let applied = |mode| AppliedOptions {
        read_preference: mode,
        ..AppliedOptions::bounded(Operation::Preview, Duration::from_secs(1))
    };
    assert!(!asks_for_non_primary(&applied(None)));
    assert!(!asks_for_non_primary(&applied(Some(
        ReadPreferenceMode::Primary
    ))));
    assert!(asks_for_non_primary(&applied(Some(
        ReadPreferenceMode::Nearest
    ))));
}
