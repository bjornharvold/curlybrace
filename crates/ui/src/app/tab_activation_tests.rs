//! Showing tabs (#159, slice 2): stale and dormant tabs, switching by position and recency,
//! reordering, closing several, and the write that settles in its own tab. Shares the helpers of
//! `tab_actions_tests`.
use super::TabAction;
use super::tests::{finds, opened, tabs, typed};
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::test_support::{
    Two, ev, orders, parked, results, two_tabs, users, with_second_session,
};
use crate::app::tabs::{Freshness, TabId};
use crate::viewmodel::{DocAction, EditorAction, FindAction};
use curlybrace_core::runtime::Event;
use curlybrace_core::store::{Namespace, Query};
use uuid::Uuid;

/// Both tabs answered, then a write to `app.orders` (the parked tab's collection) is answered.
pub(super) fn with_stale_second(t: Two) -> (AppState, TabId) {
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
fn leaving_an_empty_tab_closes_it_but_a_sessions_only_tab_stays() {
    let t = two_tabs();
    let (s, _) = tabs(t.s, TabAction::New);
    let empty = s.active_tab().expect("the empty tab");
    assert_eq!(s.tabs_of(t.a).len(), 3);
    let (s, _) = tabs(s, TabAction::Activate(t.second));
    assert!(!s.tabs.owner.contains_key(&empty), "left empty: gone");
    assert_eq!(s.tabs_of(t.a).len(), 2);
    // A session whose only tab is empty keeps it.
    let (s, _) = tabs(s, TabAction::Close(t.second));
    let first = s.active_tab().expect("first");
    let (s, _) = tabs(s, TabAction::Close(first));
    assert_eq!(s.tabs_of(t.a).len(), 1);
}

#[test]
fn the_active_tab_shortcuts_act_on_the_tab_being_looked_at() {
    let t = two_tabs();
    let first = t.s.active_tab().expect("first");
    let (s, _) = tabs(t.s, TabAction::MoveActive(1));
    assert_eq!(s.tabs.order, vec![t.second, first]);
    let (s, _) = tabs(s, TabAction::MoveActive(-5));
    assert_eq!(s.tabs.order, vec![first, t.second], "stops at the start");
    let (s, commands) = tabs(s, TabAction::DuplicateActive);
    assert_eq!(s.tabs_of(t.a).len(), 3);
    assert_eq!(finds(&commands).len(), 1);
    let copy = s.active_tab().expect("copy");
    let (s, _) = tabs(s, TabAction::CloseActive);
    assert!(!s.tabs.owner.contains_key(&copy));
    assert_eq!(s.tabs_of(t.a).len(), 2);
}

#[test]
fn close_others_and_close_right() {
    let t = two_tabs();
    let (s, _third) = opened(t.s, t.a, Namespace::new("app", "events"));
    let first = s.tabs.order[0];
    let (right, _) = tabs(s, TabAction::CloseRight(first));
    assert_eq!(
        right.tabs_of(t.a).len(),
        1,
        "everything after the first closed"
    );
    assert_eq!(right.active_tab(), Some(first));
    // The newest closed one (the last of the strip) comes back as a new tab.
    let (s, _) = tabs(right, TabAction::ReopenClosed);
    assert_eq!(s.tabs_of(t.a).len(), 2);
    assert_eq!(s.selected_ns(), Some(&orders()));
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
