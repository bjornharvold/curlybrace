//! The query options in the reducer: sent with the find, the verdict and the guard check, kept by
//! paging and every re-fetch, an answer for other options is stale, and what an option causes to
//! fail is said in the bar, not in a banner.
use super::super::explain_actions::{VerdictState, same_run, shown};
use super::super::explain_tests::{act, browsing, ns, verdict};
use super::super::reducer::{Input, update};
use super::super::state::{AppState, Banner};
use super::super::{CodeAction, CodeSource};
use crate::viewmodel::aggregate_hint_list::HINT_REQUEST_BASE;
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::findbar_more::more_open;
use crate::viewmodel::findbar_options::options_open;
use crate::viewmodel::{FindAction, SidebarAction};
use bson::doc;
use curlybrace_core::codegen::CodeRequest;
use curlybrace_core::error::CoreError;
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::pipeline::{HintOption, PipelineOptions, ReadPreferenceMode};
use curlybrace_core::runtime::{COMMAND_TIMEOUT_MESSAGE, Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::{CollectionIndex, Page, Query};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use uuid::Uuid;

fn hinted(name: &str) -> PipelineOptions {
    PipelineOptions {
        hint: Some(HintOption::Name(name.into())),
        comment: Some("why".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
        ..PipelineOptions::default()
    }
}

fn with_time(ms: u64) -> PipelineOptions {
    PipelineOptions {
        max_time_ms: Some(ms),
        ..PipelineOptions::default()
    }
}

fn query_with(options: PipelineOptions) -> Query {
    Query {
        filter: doc! { "age": 1 },
        options,
        ..Query::default()
    }
}

fn failed(error: CoreError) -> Event {
    Event::Failed {
        session: EMBEDDED_SESSION,
        context: "find",
        error,
    }
}

fn after_failure(s: AppState, error: CoreError) -> AppState {
    update(s, Input::Event(failed(error))).0
}

/// A page found with `query` is showing.
fn showing(s: AppState, query: Query) -> AppState {
    AppState {
        page: Some(Page {
            docs: vec![doc! { "_id": 1 }],
            skip: 0,
            has_more: true,
        }),
        issued_query: query.clone(),
        page_query: query,
        ..s
    }
}

#[test]
fn find_sends_the_options_in_the_query() {
    let query = query_with(hinted("a_1"));
    let (s, commands) = act(browsing(), FindAction::Run(query.clone()));
    assert_eq!(s.issued_query.options, query.options);
    let Some(Command::Find { query: sent, .. }) = commands.first() else {
        panic!("expected a find: {commands:?}");
    };
    assert_eq!(sent.options, hinted("a_1"));
}

#[test]
fn the_verdict_explain_and_the_guard_check_carry_them() {
    let query = query_with(hinted("a_1"));
    let (_, commands) = act(browsing(), FindAction::Run(query.clone()));
    let explained = commands.iter().find_map(|c| match c {
        Command::ExplainFind { query, .. } => Some(query.options.clone()),
        _ => None,
    });
    assert_eq!(explained, Some(hinted("a_1")));
    let mut guarded = browsing();
    guarded.explain.guard = true;
    let (s, commands) = act(guarded, FindAction::Run(query));
    assert!(s.guard.is_checking());
    let Some(Command::GuardCheck { targets, .. }) = commands.first() else {
        panic!("expected a guard check: {commands:?}");
    };
    assert_eq!(targets[0].query.options, hinted("a_1"));
}

#[test]
fn an_answer_for_other_options_is_stale() {
    let (s, _) = act(browsing(), FindAction::Run(query_with(hinted("a_1"))));
    let stale = Event::Results {
        session: EMBEDDED_SESSION,
        ns: ns(),
        query: query_with(hinted("b_1")),
        page: Page {
            docs: vec![doc! { "_id": 99 }],
            skip: 0,
            has_more: false,
        },
        request: 0,
    };
    let (s, _) = update(s, Input::Event(stale));
    assert_ne!(s.page_query.options, hinted("b_1"));
    assert!(s.loading_page, "the right answer is still awaited");
}

#[test]
fn a_verdict_for_other_options_is_not_shown() {
    let a = query_with(hinted("a_1"));
    let b = query_with(hinted("b_1"));
    assert!(same_run(&a, &a));
    assert!(!same_run(&a, &b));
    let s = AppState {
        verdict: VerdictState::Ready {
            query: a,
            verdict: verdict(),
        },
        ..showing(browsing(), b)
    };
    assert_eq!(shown(&s), VerdictState::None);
}

#[test]
fn refetch_after_a_write_keeps_the_options() {
    let s = showing(browsing(), query_with(hinted("a_1")));
    let (_, commands) = update(
        s,
        Input::Event(Event::Mutated {
            session: EMBEDDED_SESSION,
            ns: ns(),
        }),
    );
    let Some(Command::Find { query, .. }) = commands.first() else {
        panic!("expected a refetch: {commands:?}");
    };
    assert_eq!(query.options, hinted("a_1"));
}

#[test]
fn paging_keeps_the_options_the_shown_page_was_found_with() {
    let s = showing(browsing(), query_with(hinted("a_1")));
    // The form says something else now; the page's own query decides.
    let s = AppState {
        find: s
            .find
            .clone()
            .options_edited(OptionsEdit::Comment("later".into())),
        ..s
    };
    let (_, commands) = act(s, FindAction::Next);
    let Some(Command::Find { query, .. }) = commands.first() else {
        panic!("expected a find: {commands:?}");
    };
    assert_eq!(query.options, hinted("a_1"));
}

#[test]
fn an_option_caused_failure_is_inline_without_a_banner() {
    for error in [
        CoreError::HintNotFound,
        CoreError::CollationRejected,
        CoreError::ReadPreferenceUnavailable,
    ] {
        let s = AppState {
            loading_page: true,
            ..showing(browsing(), query_with(hinted("a_1")))
        };
        let s = after_failure(s, error.clone());
        assert_eq!(s.banner, None, "{error:?}");
        assert_eq!(s.find.last_error, Some(error));
        assert!(s.find.error_from_options);
        assert!(more_open(&s.find) && options_open(&s.find));
        assert!(!s.loading_page);
        assert!(s.page.is_some(), "the last page stays");
    }
}

#[test]
fn a_timeout_under_a_lowered_limit_is_inline_and_worded_as_yours() {
    let s = showing(browsing(), query_with(with_time(500)));
    let s = AppState {
        loading_page: true,
        ..s
    };
    let s = after_failure(s, CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into()));
    assert_eq!(s.banner, None);
    assert!(s.find.error_from_options);
    assert!(options_open(&s.find));
    assert!(!s.loading_page);
}

#[test]
fn a_plain_timeout_keeps_the_banner() {
    let timeout = CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into());
    for options in [
        PipelineOptions::default(),
        with_time(30_000),
        with_time(45_000),
    ] {
        let s = showing(browsing(), query_with(options));
        let s = after_failure(s, timeout.clone());
        assert!(matches!(s.banner, Some(Banner::Failure { .. })));
        assert_eq!(s.find.last_error, None);
        assert!(!s.find.error_from_options);
    }
}

#[test]
fn other_find_failures_keep_the_banner() {
    let s = showing(browsing(), query_with(hinted("a_1")));
    let s = after_failure(s, CoreError::Database("boom".into()));
    assert!(matches!(s.banner, Some(Banner::Failure { .. })));
    assert_eq!(s.find.last_error, None);
}

fn index(name: &str) -> CollectionIndex {
    CollectionIndex {
        spec: mock_index_spec_named(name, &[("a", KeyKind::Asc)]),
        size_bytes: None,
    }
}

#[test]
fn hint_list_wanted_from_the_find_bar_sends_one_list_indexes() {
    let (s, commands) = act(browsing(), FindAction::HintListWanted);
    let sent: Vec<u64> = commands
        .iter()
        .filter_map(|c| match c {
            Command::ListIndexes { request, .. } => Some(*request),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(sent[0] >= HINT_REQUEST_BASE);
    let (_, again) = act(s, FindAction::HintListWanted);
    assert!(again.is_empty(), "asked once");
    let mut held = browsing();
    held.indexes = held
        .indexes
        .for_collection(ns())
        .with_indexes(vec![index("a_1")]);
    let (held, commands) = act(held, FindAction::HintListWanted);
    assert!(commands.is_empty(), "the Indexes tab's list is used");
    assert_eq!(held.aggregate.options_ui.hint.names(), ["a_1"]);
}

#[test]
fn hint_list_actions_while_disconnected_show_no_banner() {
    for action in [FindAction::HintListWanted, FindAction::HintListRetry] {
        let mut s = browsing();
        s.status = super::super::state::ConnStatus::Disconnected;
        let (s, commands) = act(s, action);
        assert_eq!(s.banner, None);
        assert!(commands.is_empty());
    }
}

#[test]
fn options_work_on_a_read_only_session() {
    let mut s = browsing();
    s.read_only = true;
    let (_, commands) = act(s, FindAction::Run(query_with(hinted("a_1"))));
    assert!(matches!(commands.first(), Some(Command::Find { .. })));
}

fn typed_find(s: AppState, edits: Vec<OptionsEdit>) -> AppState {
    let mut s = s;
    s.find.text.filter = "{ \"age\": 1 }".into();
    s.find = edits
        .into_iter()
        .fold(s.find, |find, edit| find.options_edited(edit));
    s
}

#[test]
fn copy_as_code_for_find_carries_the_options() {
    let s = typed_find(
        browsing(),
        vec![
            OptionsEdit::Comment("why".into()),
            OptionsEdit::MaxTime("5000".into()),
        ],
    );
    let (s, _) = update(
        s,
        Input::Aggregate(super::super::AggregateAction::Code(CodeAction::Open(
            CodeSource::Find,
        ))),
    );
    let dialog = s.code_export.dialog.as_ref().expect("open");
    let CodeRequest::Find { query, .. } = &dialog.request else {
        panic!("expected a find request");
    };
    assert_eq!(query.options.comment.as_deref(), Some("why"));
    assert_eq!(query.options.max_time_ms, Some(5000));
}

#[test]
fn copy_as_code_is_off_while_the_form_is_blocked() {
    let s = typed_find(browsing(), vec![OptionsEdit::MaxTime("soon".into())]);
    assert_eq!(s.find.find_code_block(), Some("query.options.blocked"));
    let (s, _) = update(
        s,
        Input::Aggregate(super::super::AggregateAction::Code(CodeAction::Open(
            CodeSource::Find,
        ))),
    );
    assert_eq!(s.code_export.dialog, None);
}

#[test]
fn opening_another_collection_resets_the_options() {
    let s = typed_find(browsing(), vec![OptionsEdit::Comment("why".into())]);
    let (s, _) = update(
        s,
        Input::Sidebar(SidebarAction::Open(curlybrace_core::store::Namespace::new(
            "app", "pets",
        ))),
    );
    assert_eq!(s.find.options, Default::default());
    assert!(s.issued_query.options.is_empty());
}

#[test]
fn a_parked_session_keeps_its_options() {
    let (a, b) = (EMBEDDED_SESSION, Uuid::from_u128(77));
    let mut s = typed_find(browsing(), vec![OptionsEdit::Comment("why".into())]);
    s.open = vec![a, b];
    s.parked.insert(b, Default::default());
    let there = s.activated(b);
    assert!(there.find.options.parsed().is_ok_and(|o| o.is_empty()));
    let back = there.activated(a);
    assert_eq!(
        back.find.options.parsed().map(|o| o.comment.clone()),
        Ok(Some("why".to_string()))
    );
}
