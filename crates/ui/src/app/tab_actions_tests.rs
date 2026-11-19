//! Opening, duplicating, closing, reopening, moving and switching tabs (#159, slice 2), through
//! the reducer: no UI.
use super::{CLOSED_KEPT, MAX_TABS, TabAction, TabNotice};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::test_support::{
    act, ev, find_request, orders, parked, results, two_tabs, users, with_second_session,
};
use crate::app::tabs::{Freshness, TabId};
use crate::viewmodel::{CollectionTab, DocAction, EditorAction, SidebarAction};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Query};
use uuid::Uuid;

pub(super) fn tabs(s: AppState, action: TabAction) -> (AppState, Vec<Command>) {
    update(s, Input::Tabs(action))
}

pub(super) fn finds(commands: &[Command]) -> Vec<(Namespace, Query, u64)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::Find {
                ns, query, request, ..
            } => Some((ns.clone(), query.clone(), *request)),
            _ => None,
        })
        .collect()
}

/// Opens `ns` in a new tab of `a` and answers its find, so the tab holds a page.
pub(super) fn opened(s: AppState, a: Uuid, ns: Namespace) -> (AppState, TabId) {
    let (s, commands) = tabs(
        s,
        TabAction::OpenInNew {
            session: a,
            ns: ns.clone(),
        },
    );
    let tab = s.active_tab().expect("an active tab");
    let request = find_request(&commands);
    (ev(s, results(a, ns, request, 2)), tab)
}

pub(super) fn typed(s: AppState, tab: TabId, text: &str) -> AppState {
    s.with_tab(tab, |mut s| {
        s.find.text.filter = text.into();
        (s, Vec::new())
    })
    .0
}

#[test]
fn open_in_new_tab_creates_a_tab_and_runs_the_default_query() {
    let t = two_tabs();
    let before = t.s.tabs_of(t.a).len();
    let (s, commands) = tabs(
        t.s,
        TabAction::OpenInNew {
            session: t.a,
            ns: Namespace::new("app", "events"),
        },
    );
    assert_eq!(s.tabs_of(t.a).len(), before + 1);
    let new = s.active_tab().expect("a tab");
    assert!(new != t.second);
    assert_eq!(s.selected_ns(), Some(&Namespace::new("app", "events")));
    let sent = finds(&commands);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, Query::default());
    assert_eq!(s.issued_request, sent[0].2);
    // The tab it was opened from keeps its collection, parked.
    assert!(
        s.tabs_of(t.a)
            .iter()
            .any(|tab| s.tab_ns(*tab) == Some(users()))
    );
}

#[test]
fn plain_open_replaces_the_current_tab_s_collection() {
    let t = two_tabs();
    let before = t.s.tabs_of(t.a);
    let (s, commands) = update(t.s, Input::Sidebar(SidebarAction::Open(orders())));
    assert_eq!(s.tabs_of(t.a), before, "no tab was added");
    assert_eq!(s.selected_ns(), Some(&orders()));
    assert_eq!(finds(&commands).len(), 1);
    assert_eq!(parked(&s, t.second).ns, Some(orders()));
}

#[test]
fn the_sidebar_entry_opens_in_a_new_tab() {
    let t = two_tabs();
    let (s, commands) = update(
        t.s,
        Input::Sidebar(SidebarAction::OpenInNewTab(Namespace::new("app", "events"))),
    );
    assert_eq!(s.tabs_of(t.a).len(), 3);
    assert_eq!(finds(&commands).len(), 1);
}

#[test]
fn a_new_tab_is_empty_and_sends_nothing() {
    let t = two_tabs();
    let (s, commands) = tabs(t.s, TabAction::New);
    assert!(commands.is_empty());
    assert_eq!(s.tabs_of(t.a).len(), 3);
    assert_eq!(s.selected_ns(), None);
    assert_eq!(s.active_tab(), s.tabs.current_of(t.a));
}

#[test]
fn duplicate_copies_text_and_draft_not_results_and_takes_new_numbers() {
    let t = two_tabs();
    let first = t.s.tabs.order[0];
    let s = ev(t.s, results(t.a, orders(), t.second_find, 3));
    let s = typed(s, t.second, "{\"a\":1}");
    let (s, _) = act(s, t.second, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let source_request = parked(&s, t.second).issued_request;
    let (s, commands) = tabs(s, TabAction::Duplicate(t.second));
    let copy = s.active_tab().expect("the copy is shown");
    assert!(copy != t.second && copy != first);
    assert_eq!(s.selected_ns(), Some(&orders()));
    assert_eq!(s.find.text.filter, "{\"a\":1}");
    assert_eq!(s.tab, CollectionTab::Aggregate);
    assert_eq!(s.aggregate.stages.len(), 1, "the draft is copied");
    assert!(s.page.is_none() && s.loading_page, "no results are copied");
    let sent = finds(&commands);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].2 > source_request, "a new number");
    assert_eq!(s.issued_request, sent[0].2);
    assert!(
        s.aggregate.in_flight.is_none(),
        "nothing of the source's is awaited by the copy"
    );
    assert_eq!(parked(&s, t.second).issued_request, source_request);
}

#[test]
fn the_copy_of_a_dormant_tab_is_dormant_and_sends_nothing() {
    let t = two_tabs();
    let mut s = t.s;
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([t.second]);
    let (s, commands) = tabs(s, TabAction::Duplicate(t.second));
    assert!(commands.is_empty());
    let copy = s.active_tab().expect("a tab");
    assert_eq!(s.freshness_of(copy), Freshness::Dormant);
}

#[test]
fn close_active_focuses_the_right_neighbour() {
    let t = two_tabs();
    let first = t.s.active_tab().expect("first");
    let (s, third) = opened(t.s, t.a, Namespace::new("app", "events"));
    // Order: first, third (opened after the active tab), second. Show the first and close it.
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (s, commands) = tabs(s, TabAction::Close(first));
    assert!(commands.is_empty(), "closing sends nothing: {commands:?}");
    assert_eq!(s.active_tab(), Some(third), "the right neighbour");
    assert!(!s.tabs.owner.contains_key(&first));
    // Closing the last of the strip falls back to the left neighbour.
    let (s, _) = tabs(s, TabAction::Last);
    let last = s.active_tab().expect("last");
    let (s, _) = tabs(s, TabAction::Close(last));
    assert_eq!(s.active_tab(), Some(third));
}

#[test]
fn closing_the_last_tab_of_a_session_empties_it() {
    let t = two_tabs();
    let (s, _) = tabs(t.s, TabAction::Close(t.second));
    let only = s.active_tab().expect("a tab");
    let (s, commands) = tabs(s, TabAction::Close(only));
    assert!(commands.is_empty());
    assert_eq!(s.tabs_of(t.a), vec![only], "the session keeps one tab");
    assert_eq!(s.selected_ns(), None);
    assert!(s.page.is_none() && !s.loading_page);
    assert_eq!(s.tabs.closed.len(), 2, "both were remembered");
}

#[test]
fn close_sends_nothing_even_with_a_write_in_flight() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    let text = s
        .editor
        .as_ref()
        .map(|e| e.text.clone())
        .expect("an editor");
    let (s, saved) = update(
        s,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    assert!(matches!(saved.as_slice(), [Command::Insert { .. }]));
    let writer = s.active_tab().expect("the writer");
    let (s, commands) = tabs(s, TabAction::Close(writer));
    assert!(commands.is_empty(), "no cancel, no write: {commands:?}");
    assert!(s.pending.is_some(), "the write is still in flight");
    // Its answer frees `pending` even though its tab is gone.
    let s = ev(
        s,
        Event::Mutated {
            session: t.a,
            ns: users(),
        },
    );
    assert!(s.pending.is_none());
}

#[test]
fn reopen_closed_restores_text_and_editor_without_results() {
    let t = two_tabs();
    let mut s = ev(t.s, results(t.a, users(), t.first_find, 4));
    s.find.text.filter = "{\"a\":1}".into();
    let (s, _) = update(s, Input::Doc(DocAction::Insert));
    let typed_text = s.editor.as_ref().map(|e| e.text.clone()).expect("editor");
    let closing = s.active_tab().expect("tab");
    let (s, _) = tabs(s, TabAction::Close(closing));
    let (s, commands) = tabs(s, TabAction::ReopenClosed);
    let reopened = s.active_tab().expect("the reopened tab");
    assert!(reopened != closing, "a new tab, not the old id");
    assert_eq!(s.selected_ns(), Some(&users()));
    assert_eq!(s.find.text.filter, "{\"a\":1}");
    assert_eq!(
        s.editor.as_ref().map(|e| e.text.clone()),
        Some(typed_text),
        "the typing comes back"
    );
    assert!(s.page.is_none(), "never results");
    assert_eq!(finds(&commands).len(), 1, "one read brings the page back");
    assert!(s.tabs.closed.is_empty());
}

#[test]
fn closed_stack_keeps_five() {
    let t = two_tabs();
    let mut s = t.s;
    let mut closed = Vec::new();
    for i in 0..(CLOSED_KEPT + 2) {
        let ns = Namespace::new("app", format!("c{i}"));
        let (next, tab) = opened(s, t.a, ns.clone());
        let (next, _) = tabs(next, TabAction::Close(tab));
        s = next;
        closed.push(ns);
    }
    assert_eq!(s.tabs.closed.len(), CLOSED_KEPT);
    let newest = s.tabs.closed.last().and_then(|c| c.ns.clone());
    assert_eq!(newest, closed.last().cloned(), "the newest is last");
    let oldest = s.tabs.closed.first().and_then(|c| c.ns.clone());
    assert_eq!(
        oldest,
        Some(closed[2].clone()),
        "the oldest two were dropped"
    );
}

#[test]
fn the_21st_tab_is_refused_with_a_notice() {
    let t = two_tabs();
    let mut s = t.s;
    for i in 0..MAX_TABS {
        let ns = Namespace::new("app", format!("c{i}"));
        let (next, _) = tabs(s, TabAction::OpenInNew { session: t.a, ns });
        s = next;
    }
    assert_eq!(s.tabs.order.len(), MAX_TABS);
    for refused in [
        TabAction::New,
        TabAction::OpenInNew {
            session: t.a,
            ns: orders(),
        },
        TabAction::Duplicate(t.second),
    ] {
        let (next, commands) = tabs(s, refused);
        assert!(commands.is_empty());
        assert_eq!(next.tabs.order.len(), MAX_TABS);
        assert_eq!(next.tabs.notice, Some(TabNotice::LimitReached));
        s = next;
    }
    // Any other action clears the notice.
    let (s, _) = tabs(s, TabAction::Next);
    assert_eq!(s.tabs.notice, None);
}

#[test]
fn activating_a_tab_of_another_session_activates_that_session() {
    let t = two_tabs();
    let (s, b, b_tab) = with_second_session(t.s);
    assert_eq!(s.active, Some(b));
    let first_of_a = s.tabs.current_of(t.a).expect("a's current tab");
    let (s, commands) = tabs(s, TabAction::Activate(first_of_a));
    assert_eq!(s.active, Some(t.a));
    assert_eq!(s.active_tab(), Some(first_of_a));
    assert!(commands.is_empty(), "a fresh tab sends nothing");
    // And back to a parked tab of the other session.
    let (s, _) = tabs(s, TabAction::Activate(b_tab));
    assert_eq!(s.active, Some(b));
    assert_eq!(s.selected_ns(), Some(&orders()));
}
