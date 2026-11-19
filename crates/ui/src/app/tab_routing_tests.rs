//! An answer finds the tab that awaits its number: tab one is current and tab two is parked,
//! each with its own requests in flight (built by `tab_routing_support`).
use super::owner_of;
use super::support::{act, ev, find_request, orders, parked, results, two_tabs, users};
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::viewmodel::{CollectionTab, FindAction, IndexesAction, SidebarAction};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Query;

#[test]
fn two_tabs_on_different_collections_keep_independent_results() {
    let t = two_tabs();
    assert_ne!(t.first_find, t.second_find);
    let s = ev(t.s, results(t.a, orders(), t.second_find, 4));
    assert_eq!(
        parked(&s, t.second).page.as_ref().map(|p| p.docs.len()),
        Some(4)
    );
    assert!(s.page.is_none(), "the current tab's page did not arrive");
    assert!(s.loading_page, "and it still waits for its own answer");
    let s = ev(s, results(t.a, users(), t.first_find, 2));
    assert_eq!(s.page.as_ref().map(|p| p.docs.len()), Some(2));
    assert_eq!(
        parked(&s, t.second).page.as_ref().map(|p| p.docs.len()),
        Some(4)
    );
    assert!(!parked(&s, t.second).loading_page);
}

#[test]
fn two_tabs_on_the_same_collection_and_query_take_only_their_own_answer() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let a = cfg.id;
    let s = AppState::new(vec![cfg]);
    let (s, _) = update(s, Input::Connect(a));
    let epoch = s.epoch_counter;
    let s = ev(
        s,
        Event::Connected {
            session: a,
            epoch,
            topology: None,
        },
    );
    let (s, c1) = update(s, Input::Sidebar(SidebarAction::Open(users())));
    let (s, second) = s.tab_opened(a);
    let second = second.expect("a tab");
    let (s, c2) = act(s, second, Input::Sidebar(SidebarAction::Open(users())));
    let (r1, r2) = (find_request(&c1), find_request(&c2));
    assert_ne!(r1, r2, "the same query sent twice has two numbers");
    let s = ev(s, results(a, users(), r2, 7));
    assert_eq!(
        parked(&s, second).page.as_ref().map(|p| p.docs.len()),
        Some(7)
    );
    assert!(
        s.page.is_none(),
        "the current tab did not take the other's answer"
    );
    let s = ev(s, results(a, users(), r1, 1));
    assert_eq!(s.page.as_ref().map(|p| p.docs.len()), Some(1));
    assert_eq!(
        parked(&s, second).page.as_ref().map(|p| p.docs.len()),
        Some(7)
    );
}

#[test]
fn an_answer_for_a_closed_tab_is_dropped() {
    let t = two_tabs();
    let s = t.s.tab_closed(t.second);
    assert!(!s.parked_tabs.contains_key(&t.second));
    let before_loading = s.loading_page;
    let s = ev(s, results(t.a, orders(), t.second_find, 9));
    assert!(s.page.is_none(), "no other tab accepts it");
    assert_eq!(s.loading_page, before_loading);
    assert!(s.parked_tabs.is_empty(), "nothing leaked back");
}

#[test]
fn an_answer_nobody_awaits_goes_to_the_current_tab_as_before() {
    let t = two_tabs();
    assert_eq!(
        owner_of(&t.s, t.a, &results(t.a, users(), u64::MAX, 1)),
        None
    );
    let s = ev(t.s, results(t.a, users(), u64::MAX, 1));
    assert!(
        s.page.is_none(),
        "the current tab drops a number it did not send"
    );
    assert!(parked(&s, t.second).page.is_none());
}

#[test]
fn a_find_request_failure_goes_to_its_tab() {
    let t = two_tabs();
    let s = ev(
        t.s,
        Event::RequestFailed {
            session: t.a,
            context: "find",
            request: t.second_find,
            error: CoreError::Database("boom".into()),
        },
    );
    assert!(!parked(&s, t.second).loading_page, "tab two settled");
    assert!(s.loading_page, "tab one still waits");
}

#[test]
fn an_indexes_answer_goes_to_its_tab() {
    let t = two_tabs();
    let (s, commands) = act(t.s, t.second, Input::Tab(CollectionTab::Indexes));
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::ListIndexes { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a list request");
    assert!(parked(&s, t.second).indexes.loading);
    let s = ev(
        s,
        Event::Indexes {
            session: t.a,
            ns: orders(),
            indexes: Vec::new(),
            request,
        },
    );
    assert!(!parked(&s, t.second).indexes.loading, "tab two settled");
    let history = commands
        .iter()
        .find_map(|c| match c {
            Command::IndexHistory { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a history request");
    assert_eq!(history, request, "both reads share the tab's number");
    assert_eq!(s.indexes.request, 0, "the current tab sent nothing");
}

#[test]
fn a_hint_list_answer_goes_to_its_tab() {
    let t = two_tabs();
    let (s, commands) = act(t.s, t.second, Input::Find(FindAction::HintListWanted));
    let request = match commands.as_slice() {
        [Command::ListIndexes { request, .. }] => *request,
        other => panic!("{other:?}"),
    };
    assert!(crate::viewmodel::aggregate_hint_list::is_hint_request(
        request
    ));
    let s = ev(
        s,
        Event::Indexes {
            session: t.a,
            ns: orders(),
            indexes: Vec::new(),
            request,
        },
    );
    assert!(matches!(
        parked(&s, t.second).aggregate.options_ui.hint.list,
        crate::viewmodel::aggregate_hint_list::HintList::Loaded(_)
    ));
    assert!(
        s.aggregate.options_ui.hint.wants_list(),
        "tab one asked nothing"
    );
}

#[test]
fn a_count_answer_goes_to_its_tab() {
    let t = two_tabs();
    // The index builder's count.
    let (s, commands) = act(t.s, t.second, Input::Indexes(IndexesAction::OpenBuilder));
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::EstimatedCount { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a count request");
    let s = ev(
        s,
        Event::EstimatedCount {
            session: t.a,
            ns: orders(),
            count: 9,
            request,
        },
    );
    assert_eq!(
        parked(&s, t.second)
            .indexes
            .builder
            .as_ref()
            .and_then(|b| b.estimated_documents),
        Some(9)
    );
    assert!(s.indexes.builder.is_none());
}

#[test]
fn a_guard_answer_goes_to_its_tab() {
    let t = two_tabs();
    let (s, commands) = act(
        t.s,
        t.second,
        Input::Find(FindAction::Run(Query {
            filter: doc! { "n": 1 },
            ..Query::default()
        })),
    );
    let request = match commands.as_slice() {
        [Command::GuardCheck { request, .. }] => *request,
        other => panic!("{other:?}"),
    };
    assert!(parked(&s, t.second).guard.is_checking());
    let (s, commands) = update(
        s,
        Input::Event(Event::GuardChecked {
            session: t.a,
            request,
            outcome: Err(CoreError::Database("no plan".into())),
        }),
    );
    assert!(
        matches!(commands.first(), Some(Command::Find { ns, .. }) if *ns == orders()),
        "tab two's held-back find goes out: {commands:?}"
    );
    assert!(!parked(&s, t.second).guard.is_checking());
    assert!(s.guard.warning().is_none());
}

#[test]
fn a_query_shapes_answer_goes_to_its_tab() {
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Indexes));
    let (s, commands) = act(
        s,
        t.second,
        Input::Indexes(IndexesAction::Queries(
            crate::viewmodel::QueriesAction::Open,
        )),
    );
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::QueryShapes { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a shapes request");
    assert!(parked(&s, t.second).indexes.queries.loading);
    let s = ev(
        s,
        Event::QueryShapes {
            session: t.a,
            ns: orders(),
            range: ChartRange::Day,
            shapes: Vec::new(),
            request,
        },
    );
    assert!(!parked(&s, t.second).indexes.queries.loading);
}
