//! The hint index list in the reducer: its own numbered read, never the Indexes tab's.
use super::{is_hint_list_event, wanted};
use crate::app::AggregateAction as A;
use crate::app::OptionsAction as O;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::aggregate_hint_list::{HINT_REQUEST_BASE, HintList};
use curlybrace_core::error::CoreError;
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::{CollectionIndex, Namespace};
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;

fn people() -> Namespace {
    Namespace::new("app", "people")
}

fn connected() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(people());
    s
}

fn index(name: &str) -> CollectionIndex {
    CollectionIndex {
        spec: mock_index_spec_named(name, &[("a", KeyKind::Asc)]),
        size_bytes: None,
    }
}

fn ask(s: AppState) -> Out {
    update(s, Input::Aggregate(A::Options(O::HintListWanted)))
}

fn lists(commands: &[Command]) -> Vec<(u64, Namespace)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::ListIndexes { request, ns, .. } => Some((*request, ns.clone())),
            _ => None,
        })
        .collect()
}

fn answer(request: u64, ns: Namespace, names: &[&str]) -> Event {
    Event::Indexes {
        session: EMBEDDED_SESSION,
        ns,
        indexes: names.iter().map(|n| index(n)).collect(),
        request,
    }
}

fn failure(request: u64, error: CoreError) -> Event {
    Event::RequestFailed {
        session: EMBEDDED_SESSION,
        context: "list_indexes",
        request,
        error,
    }
}

#[test]
fn wanted_reuses_the_indexes_tab_list_for_the_same_collection() {
    let mut s = connected();
    s.indexes = s
        .indexes
        .for_collection(people())
        .with_indexes(vec![index("a_1"), index("b_1")]);
    let (s, commands) = ask(s);
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.options_ui.hint.names(), ["a_1", "b_1"]);
}

#[test]
fn wanted_sends_one_list_indexes_otherwise() {
    let (s, commands) = ask(connected());
    let sent = lists(&commands);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].0 >= HINT_REQUEST_BASE);
    assert_eq!(sent[0].1, people());
    assert!(matches!(
        s.aggregate.options_ui.hint.list,
        HintList::Loading { .. }
    ));
}

#[test]
fn wanted_twice_sends_once() {
    let (s, first) = ask(connected());
    assert_eq!(lists(&first).len(), 1);
    let (_, second) = ask(s);
    assert!(second.is_empty());
}

#[test]
fn the_answer_fills_the_hint_list_and_leaves_the_indexes_tab_alone() {
    let (s, commands) = ask(connected());
    let request = lists(&commands)[0].0;
    let before = s.indexes.clone();
    let (s, more) = update(
        s,
        Input::Event(answer(request, people(), &["a_1", "status_1"])),
    );
    assert!(more.is_empty());
    assert_eq!(s.aggregate.options_ui.hint.names(), ["a_1", "status_1"]);
    assert_eq!(s.indexes.ns, before.ns);
    assert_eq!(s.indexes.indexes, before.indexes);
    assert_eq!(s.indexes.request, before.request);
    assert_eq!(s.indexes.answered, before.answered);
    assert_eq!(s.indexes.loading, before.loading);
}

#[test]
fn a_failure_is_inline_and_raises_no_banner() {
    for error in [
        CoreError::Database("boom".into()),
        CoreError::Unreachable,
        CoreError::TlsHandshake,
    ] {
        let (s, commands) = ask(connected());
        let request = lists(&commands)[0].0;
        let (s, more) = update(s, Input::Event(failure(request, error)));
        assert!(more.is_empty());
        assert_eq!(s.aggregate.options_ui.hint.list, HintList::Failed);
        assert!(
            s.banner.is_none(),
            "{:?}",
            s.banner
                .as_ref()
                .map(|b| matches!(b, Banner::Failure { .. }))
        );
        assert_eq!(s.status, ConnStatus::Connected);
    }
}

#[test]
fn an_answer_for_another_collection_is_stale() {
    let (s, commands) = ask(connected());
    let request = lists(&commands)[0].0;
    let (s, _) = update(
        s,
        Input::Event(answer(request, Namespace::new("app", "other"), &["x"])),
    );
    assert!(matches!(
        s.aggregate.options_ui.hint.list,
        HintList::Loading { .. }
    ));
}

#[test]
fn an_indexes_tab_answer_never_reaches_the_hint_list() {
    let (s, _) = ask(connected());
    let ev = answer(5, people(), &["x"]);
    assert!(!is_hint_list_event(&ev));
    let ev = failure(5, CoreError::Unreachable);
    assert!(!is_hint_list_event(&ev));
    let (s, _) = update(s, Input::Event(answer(5, people(), &["x"])));
    assert!(matches!(
        s.aggregate.options_ui.hint.list,
        HintList::Loading { .. }
    ));
}

#[test]
fn a_collection_change_resets_the_list() {
    let (s, commands) = ask(connected());
    let request = lists(&commands)[0].0;
    let (s, _) = update(s, Input::Event(answer(request, people(), &["a_1"])));
    let draft = std::mem::take(&mut *{ s }.aggregate);
    let reset = draft.on_collection_changed();
    assert_eq!(reset.options_ui.hint.list, HintList::NotAsked);
}

#[test]
fn a_parked_session_s_answer_lands_in_its_own_draft() {
    let mut s = connected();
    let other = uuid::Uuid::new_v4();
    s.open.push(other);
    let (s, commands) = ask(s);
    let request = lists(&commands)[0].0;
    let s = s.activated(other);
    let (s, _) = update(s, Input::Event(answer(request, people(), &["a_1"])));
    let parked = &s.parked[&EMBEDDED_SESSION].aggregate;
    assert_eq!(parked.options_ui.hint.names(), ["a_1"]);
    assert_eq!(s.aggregate.options_ui.hint.list, HintList::NotAsked);
}

#[test]
fn without_a_connection_the_list_fails_and_reopening_asks_again() {
    let mut s = connected();
    s.status = ConnStatus::Disconnected;
    let (s, commands) = ask(s);
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.options_ui.hint.list, HintList::Failed);
    // Opening the section again lets it ask again; closing it keeps the failure.
    let (s, _) = update(s, Input::Aggregate(A::Options(O::Toggle)));
    assert_eq!(s.aggregate.options_ui.hint.list, HintList::NotAsked);
    let (s, _) = ask(s);
    assert_eq!(s.aggregate.options_ui.hint.list, HintList::Failed);
    let (s, _) = update(s, Input::Aggregate(A::Options(O::Toggle)));
    assert_eq!(s.aggregate.options_ui.hint.list, HintList::Failed);
    let (s, _) = update(s, Input::Aggregate(A::Options(O::Toggle)));
    assert_eq!(s.aggregate.options_ui.hint.list, HintList::NotAsked);
    let _ = wanted;
}
