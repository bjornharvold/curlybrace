//! Two tabs of one session built through the model API (no UI): tab one is current and tab two is
//! parked, each with its own find in flight. Shared by the routing tests.
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::{TabId, TabView};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

use crate::viewmodel::SidebarAction;

pub(super) fn users() -> Namespace {
    Namespace::new("app", "users")
}

pub(super) fn orders() -> Namespace {
    Namespace::new("app", "orders")
}

pub(super) fn page(n: usize) -> Page {
    Page {
        docs: (0..n).map(|i| doc! { "_id": i as i32 }).collect(),
        skip: 0,
        has_more: false,
    }
}

pub(super) struct Two {
    pub s: AppState,
    pub a: Uuid,
    pub second: TabId,
    /// The `Find` numbers each tab sent when it opened its collection.
    pub first_find: u64,
    pub second_find: u64,
}

pub(super) fn find_request(commands: &[Command]) -> u64 {
    commands
        .iter()
        .find_map(|c| match c {
            Command::Find { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a find")
}

/// Runs `input` in `tab` (the person acting on that tab).
pub(super) fn act(s: AppState, tab: TabId, input: Input) -> (AppState, Vec<Command>) {
    s.with_tab(tab, |s| update(s, input))
}

pub(super) fn ev(s: AppState, event: Event) -> AppState {
    update(s, Input::Event(event)).0
}

/// Session `a` connected; tab one (current) opened `app.users`, tab two (parked) opened
/// `app.orders`; both finds are in flight.
pub(super) fn two_tabs() -> Two {
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
    let first = s.current_tab_of(a).expect("a tab");
    let (s, first_commands) = update(s, Input::Sidebar(SidebarAction::Open(users())));
    let (s, second) = s.tab_opened(a);
    let second = second.expect("a second tab");
    let (s, second_commands) = act(s, second, Input::Sidebar(SidebarAction::Open(orders())));
    assert_eq!(s.current_tab_of(a), Some(first));
    Two {
        first_find: find_request(&first_commands),
        second_find: find_request(&second_commands),
        s,
        a,
        second,
    }
}

pub(super) fn results(session: Uuid, ns: Namespace, request: u64, n: usize) -> Event {
    Event::Results {
        session,
        ns,
        query: Query::default(),
        page: page(n),
        request,
    }
}

pub(super) fn parked(s: &AppState, tab: TabId) -> &TabView {
    s.parked_tabs.get(&tab).expect("the tab is parked")
}
