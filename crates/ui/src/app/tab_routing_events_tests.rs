//! Events no parked tab awaits go to the session's current tab, with one tab or with two.
use super::owner_of;
use super::support::two_tabs;
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::Query;
use uuid::Uuid;

use super::support::{page, users};

#[test]
fn with_one_tab_per_session_every_event_goes_to_the_current_tab() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let a = cfg.id;
    let s = AppState::new(vec![cfg]);
    let (s, _) = update(s, Input::Connect(a));
    assert_eq!(s.tabs_of(a).len(), 1);
    for event in every_event(a) {
        assert_eq!(owner_of(&s, a, &event), None, "{event:?}");
    }
}

#[test]
fn unnumbered_events_go_to_the_current_tab_even_with_two_tabs() {
    let t = two_tabs();
    for event in every_event(t.a) {
        // A number that no tab sent (u64::MAX is never drawn) and the unnumbered events: none of
        // them belongs to a parked tab.
        assert_eq!(owner_of(&t.s, t.a, &event), None, "{event:?}");
    }
}

/// One of each event a session can receive, every number one that no tab sent.
fn every_event(session: Uuid) -> Vec<Event> {
    let ns = users();
    let request = u64::MAX;
    vec![
        Event::Connected {
            session,
            epoch: 1,
            topology: None,
        },
        Event::Disconnected { session, epoch: 1 },
        Event::ConnectFailed {
            session,
            epoch: 1,
            error: CoreError::Unreachable,
        },
        Event::Databases {
            session,
            names: vec![],
        },
        Event::Collections {
            session,
            db: "app".into(),
            names: vec![],
        },
        Event::Results {
            session,
            ns: ns.clone(),
            query: Query::default(),
            page: page(0),
            request,
        },
        Event::Mutated {
            session,
            ns: ns.clone(),
        },
        Event::Failed {
            session,
            context: "find",
            error: CoreError::Unreachable,
        },
        Event::RequestFailed {
            session,
            context: "find",
            request,
            error: CoreError::Unreachable,
        },
        Event::Indexes {
            session,
            ns: ns.clone(),
            indexes: vec![],
            request,
        },
        Event::IndexHistory {
            session,
            ns: ns.clone(),
            range: ChartRange::Day,
            series: vec![],
            request,
        },
        Event::Explain {
            session,
            ns: ns.clone(),
            query: Query::default(),
            outcome: Err(CoreError::Unreachable),
            request,
        },
        Event::GuardChecked {
            session,
            request,
            outcome: Err(CoreError::Unreachable),
        },
        Event::AggregatePreviewed {
            session,
            request,
            outcome: Ok(vec![]),
        },
        Event::AggregateRan {
            session,
            run: request,
            ns: ns.clone(),
            target: ns.clone(),
            outcome: Ok(None),
        },
        Event::QueryShapes {
            session,
            ns: ns.clone(),
            range: ChartRange::Day,
            shapes: vec![],
            request,
        },
        Event::EstimatedCount {
            session,
            ns: ns.clone(),
            count: 0,
            request,
        },
        Event::IndexChanged {
            session,
            ns: ns.clone(),
        },
        Event::AdminChanged {
            session,
            db: "app".into(),
        },
        Event::BulkCount {
            session,
            ns,
            request,
            count: None,
        },
        Event::ShellHistoryChanged {
            session,
            stored: true,
        },
    ]
}

#[test]
fn a_number_the_tab_never_sent_does_not_raise_its_answered_mark() {
    let sent = crate::viewmodel::IndexesState::default().requested();
    let foreign = sent.clone().answered_request(sent.request + 1_000);
    assert_eq!(foreign.answered, 0);
    let own = sent.clone().answered_request(sent.request);
    assert_eq!(own.answered, sent.request);
}
