use super::*;
use crate::I18n;
use bson::doc;
use curlybrace_core::pipeline::{CollationOption, HintOption};

fn with(options: PipelineOptions) -> PipelineOptions {
    options
}

#[test]
fn typing_waits_for_the_pause_and_an_action_forces_it() {
    let view = AggregateView::default().options_typed(1.0);
    let (view, due) = view.take_options_due(1.2, false);
    assert!(!due, "still typing");
    assert!(view.options_due_in(1.2).is_some());
    let (view, due) = view.take_options_due(1.0 + EDIT_PAUSE_SECS, false);
    assert!(due, "paused");
    assert!(view.options_due_in(5.0).is_none());
    let (_, again) = view.take_options_due(9.0, true);
    assert!(!again, "nothing waits");
    let (_, forced) = AggregateView::default()
        .options_typed(1.0)
        .take_options_due(1.0, true);
    assert!(forced);
}

#[test]
fn an_apply_clears_the_waiting_typing() {
    let view = AggregateView::default()
        .options_typed(1.0)
        .options_applied();
    assert!(view.options_due_in(1.0).is_none());
}

#[test]
fn the_status_names_the_read_preference_and_the_hint() {
    let by_name = with(PipelineOptions {
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
        hint: Some(HintOption::Name("status_1".into())),
        ..PipelineOptions::default()
    });
    let lines = options_status(&by_name, &I18n::english());
    assert_eq!(
        lines,
        vec![
            (
                "aggregate.options.status.read_from",
                vec![("mode", "secondaryPreferred".to_string())]
            ),
            (
                "aggregate.options.status.hint",
                vec![("name", "status_1".to_string())]
            ),
        ]
    );
    let by_keys = PipelineOptions {
        hint: Some(HintOption::Keys(doc! { "a": 1, "b": -1 })),
        ..PipelineOptions::default()
    };
    assert_eq!(
        options_status(&by_keys, &I18n::english())[0].1,
        [("name", r#"{"a":1,"b":-1}"#.to_string())]
    );
    assert!(options_status(&PipelineOptions::default(), &I18n::english()).is_empty());
}

#[test]
fn the_run_lists_only_what_it_sends() {
    let all = PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: None,
            case_level: None,
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        hint: Some(HintOption::Name("a".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(1000),
        comment: Some("c".into()),
        read_preference: Some(ReadPreferenceMode::Nearest),
    };
    let labels = run_option_labels(&all);
    assert_eq!(
        labels.len(),
        5,
        "the read preference is never sent by a run"
    );
    assert!(run_option_labels(&PipelineOptions::default()).is_empty());
}

#[test]
fn the_timeout_says_you_set_it_only_under_a_lowered_limit() {
    let lowered = PipelineOptions {
        max_time_ms: Some(500),
        ..PipelineOptions::default()
    };
    let same = PipelineOptions {
        max_time_ms: Some(10_000),
        ..PipelineOptions::default()
    };
    let default = "aggregate.error.timeout";
    assert_eq!(
        timeout_key(&lowered, Operation::Preview, default),
        "aggregate.options.timeout_set"
    );
    assert_eq!(timeout_key(&same, Operation::Preview, default), default);
    assert_eq!(
        timeout_key(&PipelineOptions::default(), Operation::Explain, default),
        default
    );
}

#[test]
fn every_read_preference_has_a_key() {
    let modes = [
        None,
        Some(ReadPreferenceMode::Primary),
        Some(ReadPreferenceMode::PrimaryPreferred),
        Some(ReadPreferenceMode::Secondary),
        Some(ReadPreferenceMode::SecondaryPreferred),
        Some(ReadPreferenceMode::Nearest),
    ];
    let keys: std::collections::BTreeSet<_> = modes.into_iter().map(read_preference_key).collect();
    assert_eq!(keys.len(), 6);
}

#[test]
fn the_read_preference_in_the_status_is_the_translated_label() {
    let locale = I18n::from_json(
        r#"{"aggregate":{"options":{"read_preference":{"value":{"nearest":"le plus proche"}}}}}"#,
    )
    .expect("a locale");
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Nearest),
        ..PipelineOptions::default()
    };
    let lines = options_status(&options, &locale);
    assert_eq!(lines[0].1, [("mode", "le plus proche".to_string())]);
}
