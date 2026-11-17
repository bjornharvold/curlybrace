use super::*;
use crate::I18n;
use crate::viewmodel::findbar_more::more_open;
use crate::viewmodel::{FindAction, FindIntent, resolve_find_intent};
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{HintOption, PipelineOptions, ReadPreferenceMode};
use curlybrace_core::query::QueryText;

fn typed(filter: &str) -> FindBarState {
    FindBarState {
        text: QueryText {
            filter: filter.into(),
            ..QueryText::default()
        },
        ..FindBarState::default()
    }
}

fn edited(state: FindBarState, edits: Vec<OptionsEdit>) -> FindBarState {
    edits
        .into_iter()
        .fold(state, |state, edit| state.options_edited(edit))
}

fn run(state: &FindBarState) -> (FindBarState, Option<FindAction>) {
    resolve_find_intent(state, FindIntent::RunButton, false, false)
}

fn text(i18n: &I18n, lines: Vec<StatusLine>) -> Vec<String> {
    lines
        .into_iter()
        .map(|(key, params)| {
            let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
            i18n.t_with(key, &params)
        })
        .collect()
}

#[test]
fn the_default_bar_finds_with_no_options() {
    let (_, action) = run(&typed("{}"));
    let Some(FindAction::Run(query)) = action else {
        panic!("expected a run");
    };
    assert!(query.options.is_empty());
}

#[test]
fn find_attaches_the_parsed_options() {
    let state = edited(
        typed("{\"a\": 1}"),
        vec![
            OptionsEdit::Comment("why".into()),
            OptionsEdit::MaxTime("500".into()),
            OptionsEdit::ReadPreference(Some(ReadPreferenceMode::Nearest)),
            OptionsEdit::HintText("a_1".into()),
        ],
    );
    let Some(FindAction::Run(query)) = run(&state).1 else {
        panic!("expected a run");
    };
    assert_eq!(query.filter, doc! { "a": 1 });
    assert_eq!(
        query.options,
        PipelineOptions {
            hint: Some(HintOption::Name("a_1".into())),
            max_time_ms: Some(500),
            comment: Some("why".into()),
            read_preference: Some(ReadPreferenceMode::Nearest),
            ..PipelineOptions::default()
        }
    );
}

#[test]
fn a_blocked_form_finds_nothing_and_opens_more_and_the_section() {
    let state = edited(typed("{}"), vec![OptionsEdit::MaxTime("soon".into())]);
    assert!(state.options_blocked());
    let (next, action) = run(&state);
    assert_eq!(action, None);
    assert!(more_open(&next));
    assert!(options_open(&next));
    assert_eq!(next.options, state.options, "what was typed stays");
}

#[test]
fn a_collation_without_its_locale_waits_for_it() {
    let state = edited(typed("{}"), vec![OptionsEdit::CollationOn(true)]);
    assert!(state.options_blocked(), "the screen shows a collation");
    assert_eq!(run(&state).1, None);
}

#[test]
fn a_text_error_is_reported_before_an_options_problem() {
    let state = edited(typed("{ oops"), vec![OptionsEdit::MaxTime("0".into())]);
    let (next, action) = run(&state);
    assert_eq!(action, None);
    assert!(matches!(next.last_error, Some(CoreError::InvalidJson(_))));
}

#[test]
fn clear_resets_the_options_and_their_choice() {
    let state = edited(
        typed("{}").options_toggled(),
        vec![OptionsEdit::Comment("why".into())],
    );
    let (cleared, action) = resolve_find_intent(&state, FindIntent::ClearButton, false, false);
    assert_eq!(action, Some(FindAction::Clear));
    assert_eq!(cleared, FindBarState::default());
    assert!(!options_open(&cleared));
}

#[test]
fn more_counts_and_opens_for_set_options() {
    let none = FindBarState::default();
    assert_eq!(more_set_count(&none), 0);
    assert!(!more_open(&none));
    let state = edited(none, vec![OptionsEdit::Comment("why".into())]);
    assert_eq!(more_set_count(&state), 1);
    assert!(more_open(&state));
    let both = FindBarState {
        text: QueryText {
            sort: "{\"a\": 1}".into(),
            ..QueryText::default()
        },
        ..state
    };
    assert_eq!(more_set_count(&both), 2);
}

#[test]
fn the_option_count_counts_what_is_typed_even_when_it_does_not_parse() {
    let state = edited(
        FindBarState::default(),
        vec![
            OptionsEdit::MaxTime("x".into()),
            OptionsEdit::AllowDiskUse(crate::viewmodel::aggregate_options::Tri::On),
            OptionsEdit::CollationOn(true),
        ],
    );
    assert_eq!(options_set_count(&state.options), 3);
    let good = edited(
        FindBarState::default(),
        vec![
            OptionsEdit::Comment("why".into()),
            OptionsEdit::HintText("a_1".into()),
        ],
    );
    let parsed = good.options.parsed().expect("parses").set_count();
    assert_eq!(options_set_count(&good.options), parsed);
}

#[test]
fn the_section_settles_open_like_more() {
    let state = edited(
        FindBarState::default(),
        vec![OptionsEdit::Comment("why".into())],
    );
    assert!(options_open(&state));
    let settled = state.options_settled();
    assert_eq!(settled.options_open, Some(true));
    let emptied = settled.options_reset();
    assert!(
        options_open(&emptied),
        "deleting the values does not close it"
    );
    assert_eq!(FindBarState::default().options_settled().options_open, None);
    let closed = FindBarState {
        options_open: Some(false),
        ..emptied
    };
    assert_eq!(closed.options_settled(), closed, "a choice made is kept");
}

#[test]
fn toggling_overrides_the_automatic_state() {
    let opened = FindBarState::default().options_toggled();
    assert!(options_open(&opened));
    let set = edited(
        FindBarState::default(),
        vec![OptionsEdit::Comment("why".into())],
    );
    let closed = set.options_toggled();
    assert!(!options_open(&closed));
    assert!(options_open(&closed.options_toggled()));
}

#[test]
fn paging_keeps_the_options_of_the_shown_page() {
    use curlybrace_core::query::next_page;
    let state = edited(typed("{}"), vec![OptionsEdit::Comment("shown".into())]);
    let Some(FindAction::Run(page)) = run(&state).1 else {
        panic!("expected a run");
    };
    let edited_after = edited(state, vec![OptionsEdit::Comment("later".into())]);
    // Paging comes from the page's own query, not from the form.
    let next = next_page(&page);
    assert_eq!(next.options.comment.as_deref(), Some("shown"));
    assert_eq!(
        edited_after.options.parsed().unwrap().comment.as_deref(),
        Some("later")
    );
}

#[test]
fn the_status_names_what_shaped_the_page() {
    let i18n = I18n::english();
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
        hint: Some(HintOption::Name("status_1".into())),
        collation: Some(curlybrace_core::pipeline::CollationOption {
            locale: "en".into(),
            strength: None,
            case_level: None,
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        max_time_ms: Some(500),
        allow_disk_use: Some(true),
        ..PipelineOptions::default()
    };
    let lines = text(&i18n, page_options_status(&options, &i18n));
    assert_eq!(
        lines,
        [
            "Read from secondaryPreferred",
            "hint: status_1",
            "Collation: en",
            "Time limit: 500 ms",
            "Disk use allowed",
            "Recent writes may not show yet.",
        ]
    );
    assert!(page_options_status(&PipelineOptions::default(), &i18n).is_empty());
}

#[test]
fn a_primary_read_has_no_secondary_note_and_a_comment_has_no_line() {
    let i18n = I18n::english();
    let options = PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Primary),
        comment: Some("why".into()),
        ..PipelineOptions::default()
    };
    let lines = text(&i18n, page_options_status(&options, &i18n));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(!lines[0].contains("Recent writes"));
}

#[test]
fn a_raised_time_limit_shows_no_time_line() {
    let i18n = I18n::english();
    for ms in [30_000, 45_000] {
        let options = PipelineOptions {
            max_time_ms: Some(ms),
            ..PipelineOptions::default()
        };
        assert!(page_options_status(&options, &i18n).is_empty());
    }
}

#[test]
fn the_code_block_names_the_options_problem() {
    assert_eq!(FindBarState::default().find_code_block(), None);
    let state = edited(
        FindBarState::default(),
        vec![OptionsEdit::MaxTime("-1".into())],
    );
    assert_eq!(state.find_code_block(), Some("query.options.blocked"));
}

#[test]
fn editing_the_form_clears_an_error_the_options_caused() {
    let failed = FindBarState {
        last_error: Some(CoreError::HintNotFound),
        error_from_options: true,
        ..FindBarState::default()
    };
    let next = failed.options_edited(OptionsEdit::HintCleared);
    assert_eq!(next.last_error, None);
    assert!(!next.error_from_options);
    let text_error = FindBarState {
        last_error: Some(CoreError::InvalidJson("x".into())),
        ..FindBarState::default()
    };
    let kept = text_error.options_edited(OptionsEdit::HintCleared);
    assert!(kept.last_error.is_some(), "an error of the text stays");
}

#[test]
fn a_successful_find_clears_the_options_error_flag() {
    let state = FindBarState {
        last_error: Some(CoreError::HintNotFound),
        error_from_options: true,
        ..typed("{}")
    };
    let (next, action) = run(&state);
    assert!(action.is_some());
    assert_eq!(next.last_error, None);
    assert!(!next.error_from_options);
}

#[test]
fn the_state_prints_no_option_value() {
    let state = edited(
        FindBarState::default(),
        vec![
            OptionsEdit::Comment("secret-comment".into()),
            OptionsEdit::HintText("secret_idx".into()),
        ],
    );
    let shown = format!("{state:?}");
    assert!(!shown.contains("secret"), "{shown}");
}
