//! Tabs nobody can see (#159, review fixes): an empty tab that is not shown is swept, never
//! counted toward the limit, and a blanked tab that holds work stays visible.
use super::tests::{finds, tabs};
use super::{MAX_TABS, TabAction};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::tabs::test_support::{act, orders, parked, two_tabs, users, with_second_session};
use crate::viewmodel::CollectionTab;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::Namespace;

#[test]
fn an_empty_tab_left_by_a_session_switch_is_swept() {
    let t = two_tabs();
    let (s, _) = tabs(t.s, TabAction::New);
    let empty = s.active_tab().expect("the empty tab");
    // Another connection takes the focus without going through a tab switch.
    let (s, _b, _) = with_second_session(s);
    assert!(
        !s.tabs.owner.contains_key(&empty),
        "the empty tab of the connection left behind is gone"
    );
    assert_eq!(s.tabs_of(t.a).len(), 2);
}

#[test]
fn a_parked_tab_blanked_by_a_vanished_database_is_swept() {
    let t = two_tabs();
    let (s, _) = update(
        t.s,
        Input::Event(Event::Databases {
            session: t.a,
            names: vec!["other".into()],
        }),
    );
    assert!(
        !s.tabs.owner.contains_key(&t.second),
        "blanked and not shown: no reason to keep it"
    );
    assert_eq!(s.tabs_of(t.a).len(), 1, "the session keeps its one tab");
}

#[test]
fn a_blanked_tab_that_holds_a_draft_stays_visible() {
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (s, _) = update(
        s,
        Input::Event(Event::Databases {
            session: t.a,
            names: vec!["other".into()],
        }),
    );
    assert_eq!(parked(&s, t.second).ns, None);
    assert!(
        s.visible_tabs().contains(&t.second),
        "the draft is work: the tab is shown, and can be closed"
    );
}

#[test]
fn hidden_empty_tabs_do_not_count_toward_the_limit() {
    let t = two_tabs();
    let mut s = t.s;
    for i in 0..(MAX_TABS - 3) {
        let ns = Namespace::new("app", format!("c{i}"));
        let (next, _) = tabs(s, TabAction::OpenInNew { session: t.a, ns });
        s = next;
    }
    assert_eq!(s.visible_tabs().len(), MAX_TABS - 1);
    // A second connection whose only tab is empty (and so hidden).
    let (s, b, b_tab) = with_second_session(s);
    let (s, _) = tabs(s, TabAction::Close(b_tab));
    let lone = s.tabs.current_of(b).expect("b's tab");
    let (s, _) = tabs(s, TabAction::Close(lone));
    assert_eq!(s.tabs_of(b).len(), 1);
    let first = s.tabs_of(t.a)[0];
    let (s, _) = tabs(s, TabAction::Activate(first));
    assert_eq!(
        s.tabs.order.len(),
        MAX_TABS,
        "the hidden empty is in the strip's list"
    );
    let (s, commands) = tabs(
        s,
        TabAction::OpenInNew {
            session: t.a,
            ns: orders(),
        },
    );
    assert_eq!(finds(&commands).len(), 1, "the 20th shown tab opens");
    assert_eq!(s.visible_tabs().len(), MAX_TABS);
    let _ = users();
}
