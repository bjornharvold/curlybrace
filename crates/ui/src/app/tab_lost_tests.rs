//! A lost connection keeps every tab of the session, dormant, with its text (#159, T8).
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus};
use crate::app::tab_actions::TabAction;
use crate::app::tabs::Freshness;
use crate::app::tabs::test_support::{
    Two, act, ev, orders, parked, results, two_tabs, users, with_second_session,
};
use crate::viewmodel::{CollectionTab, DocAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use uuid::Uuid;

fn lost(session: Uuid) -> Event {
    Event::Failed {
        session,
        context: "find",
        error: CoreError::Unreachable,
    }
}

/// Both tabs hold a page and typed text; the first is on the Indexes sub-tab with a sort typed.
fn typed_two() -> Two {
    let t = two_tabs();
    let s = ev(t.s, results(t.a, users(), t.first_find, 2));
    let s = ev(s, results(t.a, orders(), t.second_find, 2));
    let (s, _) = update(s, Input::Tab(CollectionTab::Aggregate));
    let mut s = s;
    s.find.text.filter = "{\"a\":1}".into();
    let (mut s, _) = act(s, t.second, Input::Tab(CollectionTab::Documents));
    s = s
        .with_tab(t.second, |mut s| {
            s.find.text.filter = "{\"b\":2}".into();
            (s, Vec::new())
        })
        .0;
    Two { s, ..t }
}

#[test]
fn lost_connection_keeps_every_tab_dormant_with_its_text() {
    let t = typed_two();
    let first = t.s.active_tab().expect("first");
    let s = ev(t.s, lost(t.a));
    assert_eq!(s.status_of(t.a), Some(ConnStatus::Failed));
    // The tab that was current.
    assert_eq!(s.selected_ns(), Some(&users()));
    assert_eq!(s.find.text.filter, "{\"a\":1}");
    assert_eq!(s.tab, CollectionTab::Aggregate);
    assert!(s.page.is_none() && s.page_key.is_none() && !s.loading_page);
    // The parked one.
    let second = parked(&s, t.second);
    assert_eq!(second.ns, Some(orders()));
    assert_eq!(second.find.text.filter, "{\"b\":2}");
    assert!(second.page.is_none());
    for tab in [first, t.second] {
        assert_eq!(s.freshness_of(tab), Freshness::Dormant);
    }
    assert_eq!(s.tabs_of(t.a).len(), 2, "no tab was lost");
}

#[test]
fn reconnect_leaves_tabs_dormant_and_sends_only_list_databases() {
    let t = typed_two();
    let first = t.s.active_tab().expect("first");
    let s = ev(t.s, lost(t.a));
    let (s, commands) = update(s, Input::Connect(t.a));
    assert!(matches!(commands.as_slice(), [Command::Connect(_)]));
    let epoch = s.epoch_counter;
    let (s, commands) = update(
        s,
        Input::Event(Event::Connected {
            session: t.a,
            epoch,
            topology: None,
        }),
    );
    assert!(
        commands
            .iter()
            .all(|c| matches!(c, Command::ListDatabases { .. })),
        "nothing is read until Run: {commands:?}"
    );
    assert_eq!(s.selected_ns(), Some(&users()), "the collection survived");
    assert_eq!(s.find.text.filter, "{\"a\":1}");
    assert_eq!(parked(&s, t.second).ns, Some(orders()));
    assert_eq!(s.freshness_of(first), Freshness::Dormant);
    assert_eq!(s.freshness_of(t.second), Freshness::Dormant);
    // Activating the other tab still reads nothing; its Run does.
    let (s, commands) = update(s, Input::Tabs(TabAction::Activate(t.second)));
    assert!(commands.is_empty());
    let mut s = s;
    s.explain.guard = false;
    let (_, commands) = update(s, Input::Tabs(TabAction::Run(t.second)));
    assert!(commands.iter().any(|c| matches!(c, Command::Find { .. })));
}

#[test]
fn a_find_after_reconnect_wakes_the_current_tab_only() {
    let t = typed_two();
    let first = t.s.active_tab().expect("first");
    let s = ev(t.s, lost(t.a));
    let (s, _) = update(s, Input::Connect(t.a));
    let epoch = s.epoch_counter;
    let s = ev(
        s,
        Event::Connected {
            session: t.a,
            epoch,
            topology: None,
        },
    );
    let (s, _) = update(s, Input::Tab(CollectionTab::Documents));
    let mut s = s;
    s.explain.guard = false;
    let (s, commands) = update(
        s,
        Input::Find(crate::viewmodel::FindAction::Run(Default::default())),
    );
    assert!(commands.iter().any(|c| matches!(c, Command::Find { .. })));
    assert_eq!(s.freshness_of(first), Freshness::Fresh);
    assert_eq!(s.freshness_of(t.second), Freshness::Dormant);
}

#[test]
fn another_session_s_tabs_are_not_dormant() {
    let t = typed_two();
    let (s, b, b_tab) = with_second_session(t.s);
    let s = ev(s, lost(t.a));
    assert_eq!(s.status_of(b), Some(ConnStatus::Connected));
    assert_ne!(s.freshness_of(b_tab), Freshness::Dormant);
    assert_eq!(s.freshness_of(t.second), Freshness::Dormant);
}

#[test]
fn a_forgotten_editor_in_a_hidden_tab_keeps_the_session_and_says_so() {
    // Slice 1's rule stays exact: an editor in any tab keeps the session through one blip.
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Doc(DocAction::Insert));
    let s: AppState = ev(s, lost(t.a));
    assert_eq!(s.status_of(t.a), Some(ConnStatus::Connected));
    let glance = s.glance(t.second).expect("a glance");
    assert!(glance.unsaved, "the tab says it holds unsaved changes");
    let other = s.active_tab().expect("first");
    assert!(!s.glance(other).expect("glance").unsaved);
}
