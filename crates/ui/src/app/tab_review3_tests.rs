//! Fixes after the code review of PR #188 (#159): work in a blank tab survives leaving it, a stale
//! tab is read again wherever it becomes the shown one, Duplicate takes no editor, Close others
//! stays in its connection, the limit notice does not linger.
use super::activation_tests::with_stale_second;
use super::tests::{finds, tabs};
use super::{MAX_TABS, TabAction, TabNotice};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::tabs::Freshness;
use crate::app::tabs::test_support::{parked, two_tabs, with_second_session};
use crate::viewmodel::{CollectionTab, DocAction, SidebarAction};
use curlybrace_core::store::Namespace;

#[test]
fn a_blank_tab_holding_an_aggregate_draft_survives_leaving_it() {
    let t = two_tabs();
    let (s, _) = tabs(t.s, TabAction::New);
    let blank = s.active_tab().expect("the new tab");
    let (s, _) = update(s, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    assert_eq!(s.selected_ns(), None);
    let (s, _) = tabs(s, TabAction::Activate(t.second));
    assert!(
        s.tabs.owner.contains_key(&blank),
        "a draft is work: leaving the tab does not throw it away"
    );
    assert_eq!(parked(&s, blank).aggregate.stages.len(), 1);
    assert!(s.visible_tabs().contains(&blank));
}

#[test]
fn a_blank_tab_holding_an_editor_survives_leaving_it() {
    let t = two_tabs();
    let (s, _) = tabs(t.s, TabAction::New);
    let blank = s.active_tab().expect("the new tab");
    // An editor can be open on a tab whose collection was dropped.
    let mut s = s;
    s.editor = Some(crate::viewmodel::EditorState::for_new());
    let (s, _) = tabs(s, TabAction::Activate(t.second));
    assert!(s.tabs.owner.contains_key(&blank));
}

#[test]
fn a_stale_tab_that_became_current_in_the_background_is_read_when_its_connection_is_shown() {
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let (s, _b, _) = with_second_session(s);
    // a's current tab closes in the background: the stale one takes over (its mark goes with it).
    let first = s.tabs.current_of(t_session(&s, second)).expect("current");
    let (s, commands) = tabs(s, TabAction::Close(first));
    assert!(commands.is_empty());
    let a = t_session(&s, second);
    // Back to connection a from the sidebar.
    let (s, commands) = update(s, Input::Activate(a));
    let sent = finds(&commands);
    assert_eq!(sent.len(), 1, "the shown tab reads again: {commands:?}");
    assert_eq!(sent[0].0, super::super::tabs::test_support::orders());
    assert_eq!(s.freshness_of(second), Freshness::Fresh);
}

#[test]
fn closing_the_shown_connection_reads_the_stale_tab_of_the_one_it_reveals() {
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let a = t_session(&s, second);
    // Another connection comes in front; a's current tab closes behind it, so the stale one takes
    // over in the background. Closing b then reveals a.
    let (s, b, _) = with_second_session(s);
    let first = s.tabs.current_of(a).expect("current");
    let (s, _) = tabs(s, TabAction::Close(first));
    let (s, commands) = update(s, Input::Disconnect(b));
    assert_eq!(s.active, Some(a));
    assert_eq!(finds(&commands).len(), 1, "{commands:?}");
}

#[test]
fn a_sidebar_click_on_another_connection_reads_its_stale_tab() {
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let a = t_session(&s, second);
    let (s, _b, _) = with_second_session(s);
    let first = s.tabs.current_of(a).expect("current");
    let (s, _) = tabs(s, TabAction::Close(first));
    let (s, commands) = update(
        s,
        Input::Sidebar(SidebarAction::InSession {
            session: a,
            action: Box::new(SidebarAction::Refresh),
        }),
    );
    assert_eq!(s.active, Some(a));
    assert!(
        finds(&commands).len() == 1,
        "a connection shown by a click on its tree reads its stale tab: {commands:?}"
    );
}

#[test]
fn close_others_reveals_a_stale_tab_with_a_read() {
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let (s, commands) = tabs(s, TabAction::CloseOthers(second));
    assert_eq!(finds(&commands).len(), 1);
    assert_eq!(s.active_tab(), Some(second));
}

#[test]
fn a_duplicate_takes_no_editor_but_a_reopened_tab_does() {
    let t = two_tabs();
    let source = t.s.active_tab().expect("source");
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    assert!(s.editor.is_some());
    let (s, _) = tabs(s, TabAction::Duplicate(source));
    assert!(s.editor.is_none(), "two editors on one document: no");
    assert!(
        parked(&s, source).editor.is_some(),
        "the source keeps its own"
    );
    // The plan's rule for a closed tab stays: it comes back with its editor.
    let (s, _) = tabs(s, TabAction::Close(source));
    let (s, _) = tabs(s, TabAction::ReopenClosed);
    assert!(s.editor.is_some());
}

/// Two connections with two tabs each, the first connection's shown. Returns the state, the two
/// connections and the tabs `(a1, a2, b1, b2)`.
fn four() -> (
    crate::app::state::AppState,
    uuid::Uuid,
    uuid::Uuid,
    [crate::app::tabs::TabId; 4],
) {
    let t = two_tabs();
    let a1 = t.s.active_tab().expect("a1");
    let (s, b, b2) = with_second_session(t.s);
    let b1 = s.tabs.current_of(b).expect("b1");
    let (s, _) = tabs(s, TabAction::Activate(a1));
    (s, t.a, b, [a1, t.second, b1, b2])
}

#[test]
fn close_others_closes_only_the_tabs_of_its_own_connection() {
    let (s, a, b, [a1, a2, b1, b2]) = four();
    let (s, _) = tabs(s, TabAction::CloseOthers(a1));
    assert!(
        !s.tabs.owner.contains_key(&a2),
        "its own connection's tab went"
    );
    assert!(
        s.tabs.owner.contains_key(&b1) && s.tabs.owner.contains_key(&b2),
        "the other connection's tabs are not its business"
    );
    assert_eq!(s.tabs_of(b).len(), 2);
    assert_eq!(s.tabs.closed.len(), 1, "only a2 was remembered");
    let _ = a;
}

#[test]
fn close_to_the_right_closes_only_the_tabs_of_its_own_connection() {
    let (s, _a, b, [a1, a2, b1, b2]) = four();
    let (s, _) = tabs(s, TabAction::CloseRight(a1));
    assert!(!s.tabs.owner.contains_key(&a2));
    assert!(s.tabs.owner.contains_key(&b1) && s.tabs.owner.contains_key(&b2));
    assert_eq!(s.tabs_of(b).len(), 2);
    assert_eq!(s.tabs.closed.len(), 1);
}

#[test]
fn the_limit_notice_clears_on_the_next_input_that_is_not_a_tab_action() {
    let t = two_tabs();
    let mut s = t.s;
    for i in 0..MAX_TABS {
        let ns = Namespace::new("app", format!("c{i}"));
        let (next, _) = tabs(s, TabAction::OpenInNew { session: t.a, ns });
        s = next;
    }
    let (s, _) = tabs(s, TabAction::New);
    assert_eq!(s.tabs.notice, Some(TabNotice::LimitReached));
    // Another refused tab action keeps it as it is (announced once, not again).
    let (s, _) = tabs(s, TabAction::New);
    assert_eq!(s.tabs.notice, Some(TabNotice::LimitReached));
    // Anything else the person does dismisses it.
    let (s, _) = update(s, Input::DismissBanner);
    assert_eq!(s.tabs.notice, None);
}

fn t_session(s: &crate::app::state::AppState, tab: crate::app::tabs::TabId) -> uuid::Uuid {
    s.tabs.owner.get(&tab).copied().expect("an owner")
}
