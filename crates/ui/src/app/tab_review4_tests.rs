//! Fixes after the second review of PR #188 (#159): answers do not dismiss the limit notice, a
//! click that replaces or leaves the shown tab does not read it first, a stale mark stays when no
//! read was sent, a connection picked from the connection list reads its stale tab.
use super::activation_tests::with_stale_second;
use super::tests::{finds, tabs};
use super::{MAX_TABS, TabAction, TabNotice};
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{orders, two_tabs, with_second_session};
use crate::app::tools_state::{ProbeScope, ToolsAction};
use crate::viewmodel::SidebarAction;
use curlybrace_core::store::Namespace;
use curlybrace_core::tools::{ToolError, ToolKind};
use uuid::Uuid;

/// Connection `a` has a stale tab that became its current one in the background, while another
/// connection is shown.
fn stale_in_the_background() -> (AppState, Uuid, TabId) {
    let t = two_tabs();
    let (s, second) = with_stale_second(t);
    let (s, _b, _) = with_second_session(s);
    let a = s.tabs.owner.get(&second).copied().expect("an owner");
    let first = s.tabs.current_of(a).expect("current");
    let (s, commands) = tabs(s, TabAction::Close(first));
    assert!(commands.is_empty());
    assert!(s.tabs.stale.contains(&second));
    (s, a, second)
}

fn at_the_limit() -> AppState {
    let t = two_tabs();
    let mut s = t.s;
    for i in 0..MAX_TABS {
        let ns = Namespace::new("app", format!("c{i}"));
        let (next, _) = tabs(s, TabAction::OpenInNew { session: t.a, ns });
        s = next;
    }
    let (s, _) = tabs(s, TabAction::New);
    assert_eq!(s.tabs.notice, Some(TabNotice::LimitReached));
    s
}

#[test]
fn an_answer_does_not_dismiss_the_limit_notice_but_a_person_does() {
    let s = at_the_limit();
    let probed = ToolsAction::Probed {
        scope: ProbeScope::Global,
        kind: ToolKind::Mongosh,
        request: 0,
        result: Err(ToolError::Missing),
    };
    let (s, _) = update(s, Input::Tools(probed));
    assert_eq!(
        s.tabs.notice,
        Some(TabNotice::LimitReached),
        "a probe answer"
    );
    let (s, _) = update(s, Input::ForgetFinished(Ok(())));
    assert_eq!(
        s.tabs.notice,
        Some(TabNotice::LimitReached),
        "a forget answer"
    );
    let (s, _) = update(s, Input::Tools(ToolsAction::Browsed { path: None }));
    assert_eq!(
        s.tabs.notice,
        Some(TabNotice::LimitReached),
        "a picker answer"
    );
    let (s, _) = update(s, Input::DismissBanner);
    assert_eq!(s.tabs.notice, None, "the person did something else");
}

#[test]
fn opening_in_a_new_tab_does_not_read_the_stale_tab_it_leaves_behind() {
    let (s, a, _second) = stale_in_the_background();
    let events = Namespace::new("app", "events");
    let (_, commands) = update(
        s,
        Input::Sidebar(SidebarAction::InSession {
            session: a,
            action: Box::new(SidebarAction::OpenInNewTab(events.clone())),
        }),
    );
    let sent = finds(&commands);
    assert_eq!(sent.len(), 1, "only the new tab reads: {commands:?}");
    assert_eq!(sent[0].0, events);
}

#[test]
fn comparing_from_a_connections_tree_does_not_read_its_stale_tab() {
    let (s, a, _second) = stale_in_the_background();
    let (_, commands) = update(
        s,
        Input::Sidebar(SidebarAction::InSession {
            session: a,
            action: Box::new(SidebarAction::CompareTo(orders())),
        }),
    );
    assert!(finds(&commands).is_empty(), "{commands:?}");
}

#[test]
fn a_stale_mark_stays_when_the_collection_is_gone_and_no_read_was_sent() {
    let (mut s, a, second) = stale_in_the_background();
    // The shown tab's collection vanished in the meantime.
    if let Some(view) = s.parked.get_mut(&a) {
        view.sidebar.selected = None;
    }
    let (s, commands) = update(s, Input::Activate(a));
    assert!(finds(&commands).is_empty());
    assert!(
        s.tabs.stale.contains(&second),
        "no read was sent: the tab is still out of date"
    );
}

#[test]
fn connecting_an_open_connection_reads_its_stale_tab() {
    let (s, a, second) = stale_in_the_background();
    // The connection list's Connect on a connection that is already open just shows it.
    let (s, commands) = update(s, Input::Connect(a));
    assert_eq!(s.active, Some(a));
    assert_eq!(finds(&commands).len(), 1, "{commands:?}");
    assert!(!s.tabs.stale.contains(&second));
}
