//! Opening, duplicating, closing, reopening, moving and switching tabs (#159, slice 2), through
//! the reducer: no UI.
use super::{CLOSED_KEPT, MAX_TABS, TabAction, TabNotice};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::test_support::{
    Two, act, ev, find_request, orders, parked, results, two_tabs, users, with_second_session,
};
use crate::app::tabs::{Freshness, TabId};
use crate::viewmodel::{CollectionTab, DocAction, EditorAction, FindAction, SidebarAction};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Query};
use uuid::Uuid;

fn tabs(s: AppState, action: TabAction) -> (AppState, Vec<Command>) {
    update(s, Input::Tabs(action))
}

fn finds(commands: &[Command]) -> Vec<(Namespace, Query, u64)> {
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
fn opened(s: AppState, a: Uuid, ns: Namespace) -> (AppState, TabId) {
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

fn typed(s: AppState, tab: TabId, text: &str) -> AppState {
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
    for _ in 0..MAX_TABS {
        let (next, _) = tabs(s, TabAction::New);
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

/// Both tabs answered, then a write to `app.orders` (the parked tab's collection) is answered.
fn with_stale_second(t: Two) -> (AppState, TabId) {
    let s = ev(t.s, results(t.a, users(), t.first_find, 1));
    let s = ev(s, results(t.a, orders(), t.second_find, 1));
    let s = ev(
        s,
        Event::Mutated {
            session: t.a,
            ns: orders(),
        },
    );
    assert_eq!(parked(&s, t.second).freshness, Freshness::Stale);
    (s, t.second)
}

#[test]
fn activating_a_stale_tab_refetches_its_page() {
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let (s, commands) = tabs(s, TabAction::Activate(second));
    let sent = finds(&commands);
    assert_eq!(sent.len(), 1, "one re-read: {commands:?}");
    assert_eq!(sent[0].0, orders());
    assert_eq!(s.freshness_of(second), Freshness::Fresh);
    assert_eq!(s.issued_request, sent[0].2);
    // Showing it again re-reads nothing.
    let first = s.tabs.order[0];
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (_, commands) = tabs(s, TabAction::Activate(second));
    assert!(commands.is_empty());
}

#[test]
fn a_routed_answer_to_a_stale_parked_tab_keeps_it_stale() {
    // A find sent before the write can be answered after it, so the answer proves nothing: the
    // tab is re-read when shown (a spare read is fine, a missed one is not).
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let session = s.active.expect("an active session");
    let awaited = parked(&s, second).issued_request;
    let s = ev(s, results(session, orders(), awaited, 2));
    assert_eq!(
        parked(&s, second).page.as_ref().map(|p| p.docs.len()),
        Some(2)
    );
    assert_eq!(parked(&s, second).freshness, Freshness::Stale);
    let (_, commands) = tabs(s, TabAction::Activate(second));
    assert_eq!(finds(&commands).len(), 1);
}

#[test]
fn closing_the_current_tab_re_reads_the_stale_tab_it_reveals() {
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    // The parked, stale tab takes the closed one's place: showing it is an activation, and the
    // close itself sends nothing.
    let first = s.active_tab().expect("first");
    let (s, commands) = tabs(s, TabAction::Close(first));
    assert_eq!(s.active_tab(), Some(second));
    let sent = finds(&commands);
    assert_eq!(sent.len(), 1);
    assert_eq!(commands.len(), 1, "only that read: {commands:?}");
    assert_eq!(sent[0].0, orders());
    assert_eq!(s.freshness_of(second), Freshness::Fresh);
}

#[test]
fn the_stale_mark_survives_becoming_current_without_being_shown() {
    // Two sessions: a's current tab closes while b is in front, so a's stale parked tab becomes
    // a's current tab in the background. Its mark must go with it.
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let (s, _b, _) = with_second_session(s);
    let first = s.tabs.current_of(t_session(&s, second)).expect("current");
    let (s, commands) = tabs(s, TabAction::Close(first));
    assert!(commands.is_empty(), "a background close sends nothing");
    assert_eq!(s.tabs.current_of(t_session(&s, second)), Some(second));
    assert_eq!(s.freshness_of(second), Freshness::Stale);
    let (_, commands) = tabs(s, TabAction::Activate(second));
    assert_eq!(finds(&commands).len(), 1, "it still re-reads when shown");
}

fn t_session(s: &AppState, tab: TabId) -> Uuid {
    s.tabs.owner.get(&tab).copied().expect("an owner")
}

#[test]
fn activating_a_dormant_tab_sends_nothing() {
    let t = two_tabs();
    let mut s = t.s;
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([t.second]);
    let (s, commands) = tabs(s, TabAction::Activate(t.second));
    assert!(commands.is_empty());
    assert_eq!(s.active_tab(), Some(t.second));
    assert_eq!(
        s.freshness_of(t.second),
        Freshness::Dormant,
        "still waiting for Run"
    );
}

#[test]
fn run_on_a_dormant_tab_sends_its_find() {
    let t = two_tabs();
    let mut s = typed(t.s, t.second, "{\"a\":1}");
    s.explain.guard = false;
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([t.second]);
    let (s, commands) = tabs(s, TabAction::Run(t.second));
    let sent = finds(&commands);
    assert_eq!(sent.len(), 1, "{commands:?}");
    assert_eq!(sent[0].0, orders());
    assert_eq!(sent[0].1.filter, bson::doc! { "a": 1 });
    assert_eq!(s.freshness_of(t.second), Freshness::Fresh);
}

#[test]
fn a_find_from_the_bar_wakes_a_dormant_tab() {
    let t = two_tabs();
    let mut s = t.s;
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([t.second]);
    let (s, _) = tabs(s, TabAction::Activate(t.second));
    let (s, commands) = update(s, Input::Find(FindAction::Run(Query::default())));
    assert_eq!(finds(&commands).len(), 1);
    assert_eq!(s.freshness_of(t.second), Freshness::Fresh);
}

#[test]
fn next_previous_mru_nth_last() {
    let t = two_tabs();
    let (s, third) = opened(t.s, t.a, Namespace::new("app", "events"));
    // Strip order: first, third, second; the third is shown.
    let first = s.tabs.order[0];
    assert_eq!(s.tabs.order, vec![first, third, t.second]);
    let (s, _) = tabs(s, TabAction::Next);
    assert_eq!(s.active_tab(), Some(t.second));
    let (s, _) = tabs(s, TabAction::Next);
    assert_eq!(s.active_tab(), Some(first), "wraps around");
    let (s, _) = tabs(s, TabAction::Previous);
    assert_eq!(s.active_tab(), Some(t.second));
    let (s, _) = tabs(s, TabAction::Nth(1));
    assert_eq!(s.active_tab(), Some(third));
    let (s, _) = tabs(s, TabAction::Last);
    assert_eq!(s.active_tab(), Some(t.second));
    let (s, _) = tabs(s, TabAction::Nth(9));
    assert_eq!(
        s.active_tab(),
        Some(t.second),
        "out of range changes nothing"
    );
    // Most recently used: the one shown just before this one (the third), then the oldest.
    let (s, _) = tabs(s, TabAction::NextMru);
    assert_eq!(s.active_tab(), Some(third));
    let (s, _) = tabs(s, TabAction::NextMru);
    assert_eq!(
        s.active_tab(),
        Some(t.second),
        "toggles between the last two"
    );
    let (s, _) = tabs(s, TabAction::PreviousMru);
    assert_eq!(s.active_tab(), Some(first), "the least recently used");
}

#[test]
fn move_reorders() {
    let t = two_tabs();
    let (s, third) = opened(t.s, t.a, Namespace::new("app", "events"));
    let first = s.tabs.order[0];
    let (s, _) = tabs(s, TabAction::Move { tab: first, to: 2 });
    assert_eq!(s.tabs.order, vec![third, t.second, first]);
    let (s, _) = tabs(s, TabAction::Move { tab: first, to: 0 });
    assert_eq!(s.tabs.order, vec![first, third, t.second]);
    let (s, _) = tabs(s, TabAction::Move { tab: first, to: 99 });
    assert_eq!(
        s.tabs.order,
        vec![third, t.second, first],
        "clamped to the end"
    );
}

#[test]
fn close_others_and_close_right() {
    let t = two_tabs();
    let (s, third) = opened(t.s, t.a, Namespace::new("app", "events"));
    let first = s.tabs.order[0];
    let (right, _) = tabs(s, TabAction::CloseRight(first));
    assert_eq!(
        right.tabs_of(t.a).len(),
        1,
        "everything after the first closed"
    );
    assert_eq!(right.active_tab(), Some(first));
    let (s, _) = tabs(right, TabAction::ReopenClosed);
    let _ = (s, third);
    let t = two_tabs();
    let (s, _) = opened(t.s, t.a, Namespace::new("app", "events"));
    let keep = s.tabs.order[1];
    let (s, _) = tabs(s, TabAction::CloseOthers(keep));
    assert_eq!(s.active_tab(), Some(keep));
    assert_eq!(s.visible_tabs(), vec![keep]);
}

#[test]
fn a_write_settles_in_its_own_tab_with_two_tabs_on_the_same_collection() {
    // Both tabs show app.users; the write is saved from the parked one.
    let t = two_tabs();
    let (s, same) = opened(t.s, t.a, users());
    let (s, _) = update(s, Input::Doc(DocAction::Insert));
    let text = s.editor.as_ref().map(|e| e.text.clone()).expect("editor");
    let (s, _) = update(
        s,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    // The person looks at the other tab on the same collection while it saves.
    let first = t_first(&s, same);
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (s, commands) = update(
        s,
        Input::Event(Event::Mutated {
            session: t.a,
            ns: users(),
        }),
    );
    assert!(s.pending.is_none());
    assert_eq!(finds(&commands).len(), 1, "the shown tab re-reads");
    assert!(
        parked(&s, same).editor.is_none(),
        "the writer's editor is closed in the writer"
    );
    assert_eq!(
        parked(&s, same).freshness,
        Freshness::Stale,
        "and it re-reads when shown"
    );
}

fn t_first(s: &AppState, not: TabId) -> TabId {
    s.tabs
        .order
        .iter()
        .copied()
        .find(|t| *t != not && s.tab_ns(*t) == Some(users()))
        .expect("the other tab on users")
}
