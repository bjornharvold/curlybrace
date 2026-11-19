//! Every tab's reads draw their numbers from one process-wide source (#159, slice 1): no two
//! states, tabs or features ever send the same number, which is what lets an answer find the one
//! state that waits for it.
use super::next;
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::{AggregateState, FindAction, IndexesState, QueriesState};
use bson::doc;
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use std::collections::BTreeSet;

#[test]
fn numbers_never_repeat_across_threads() {
    let taken: Vec<u64> = (0..8)
        .map(|_| std::thread::spawn(|| (0..1000).map(|_| next()).collect::<Vec<_>>()))
        .collect::<Vec<_>>()
        .into_iter()
        .flat_map(|t| t.join().expect("a thread"))
        .collect();
    let unique: BTreeSet<u64> = taken.iter().copied().collect();
    assert_eq!(unique.len(), 8000, "no number was handed out twice");
    assert!(!unique.contains(&0), "0 is never a request number");
}

#[test]
fn two_index_states_never_send_the_same_number() {
    let a = IndexesState::default().requested();
    let b = IndexesState::default().requested();
    assert_ne!(a.request, b.request);
    assert!(b.request > a.request, "a later request has a larger number");
}

#[test]
fn two_query_shape_states_never_send_the_same_number() {
    let a = QueriesState::default().requested();
    let b = QueriesState::default().requested();
    assert_ne!(a.request, b.request);
}

#[test]
fn two_drafts_never_send_the_same_preview_number() {
    let preview = |_: ()| {
        let (_, sent) = AggregateState::default()
            .add_stage("$match")
            .begin_preview();
        sent.expect("a preview is sent").0
    };
    assert_ne!(preview(()), preview(()));
}

fn users() -> Namespace {
    Namespace::new("app", "users")
}

/// A connected embedded session with `app.users` open and its first page shown.
fn browsing() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.explain.guard = false;
    s.explain.automatic = false;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["users".into()])
        .toggled("app")
        .selecting(users());
    s
}

fn results(request: u64, query: Query, n: usize) -> Event {
    Event::Results {
        session: EMBEDDED_SESSION,
        ns: users(),
        query,
        page: Page {
            docs: (0..n).map(|i| doc! { "_id": i as i32 }).collect(),
            skip: 0,
            has_more: false,
        },
        request,
    }
}

fn sent_request(commands: &[Command]) -> u64 {
    commands
        .iter()
        .find_map(|c| match c {
            Command::Find { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a find was sent")
}

#[test]
fn a_results_answer_for_an_older_identical_query_is_stale() {
    let query = Query {
        filter: doc! { "a": 1 },
        ..Query::default()
    };
    let (s, first) = update(browsing(), Input::Find(FindAction::Run(query.clone())));
    let first = sent_request(&first);
    let (s, second) = update(s, Input::Find(FindAction::Run(query.clone())));
    let second = sent_request(&second);
    assert!(second > first, "the newer find has the larger number");
    // The older find's answer arrives last: it describes the same query but not the newest ask.
    let (s, _) = update(s, Input::Event(results(second, query.clone(), 2)));
    assert_eq!(s.page.as_ref().map(|p| p.docs.len()), Some(2));
    let (s, _) = update(s, Input::Event(results(first, query.clone(), 7)));
    assert_eq!(
        s.page.as_ref().map(|p| p.docs.len()),
        Some(2),
        "the older identical query is stale"
    );
}

#[test]
fn the_newest_answer_is_accepted_even_when_the_older_one_never_arrives() {
    let query = Query {
        filter: doc! { "a": 1 },
        ..Query::default()
    };
    let (s, _) = update(browsing(), Input::Find(FindAction::Run(query.clone())));
    let (s, second) = update(s, Input::Find(FindAction::Run(query.clone())));
    let second = sent_request(&second);
    let (s, _) = update(s, Input::Event(results(second, query, 3)));
    assert_eq!(s.page.as_ref().map(|p| p.docs.len()), Some(3));
    assert!(!s.loading_page);
}
