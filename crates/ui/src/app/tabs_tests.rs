//! The tab model: the swap, the strip it keeps in step with the sessions, and `with_tab`.
use super::{TabId, TabView};
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus, Pending};
use crate::viewmodel::{
    AggregateState, CollectionTab, EditorState, FindBarState, IndexesState, PageKey, SidebarAction,
    ViewMode,
};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

fn app(names: &[&str]) -> (AppState, Vec<Uuid>) {
    let cfgs: Vec<_> = names
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids = cfgs.iter().map(|c| c.id).collect();
    (AppState::new(cfgs), ids)
}

/// Connects `id` and answers the Connect, so the session is active and `Connected`.
fn opened(state: AppState, id: Uuid) -> AppState {
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    )
    .0
}

fn users() -> Namespace {
    Namespace::new("app", "users")
}

fn page() -> Page {
    Page {
        docs: vec![doc! { "_id": 1, "who": "ada" }],
        skip: 0,
        has_more: true,
    }
}

/// A foreground whose every tab-level field holds something other than its default.
fn busy(state: AppState) -> AppState {
    let mut s = state;
    s.sidebar.selected = Some(users());
    s.find = FindBarState {
        text: curlybrace_core::query::QueryText {
            filter: "{\"a\": 1}".into(),
            ..Default::default()
        },
        ..FindBarState::default()
    };
    s.issued_query = Query {
        skip: 3,
        ..Query::default()
    };
    s.issued_request = 41;
    s.verdict_request = 42;
    s.page_query = Query {
        skip: 5,
        ..Query::default()
    };
    s.loading_page = true;
    s.page = Some(page());
    s.page_key = Some(PageKey {
        len: 1,
        skip: 0,
        fingerprint: 7,
        ..PageKey::default()
    });
    s.verdict = crate::app::VerdictState::Pending {
        query: Query::default(),
    };
    s.guard = Box::new(crate::app::GuardState::Checking {
        request: 9,
        query: Query::default(),
    });
    s.view_mode = ViewMode::Json;
    s.tab = CollectionTab::Aggregate;
    s.indexes = IndexesState {
        ns: Some(users()),
        selected: Some("idx".into()),
        ..IndexesState::default()
    };
    s.aggregate = Box::new(AggregateState::default().add_stage("$match"));
    s.editor = Some(EditorState::for_new());
    s
}

#[test]
fn take_then_restore_is_identity() {
    let (s, ids) = app(&["a"]);
    let mut s = busy(opened(s, ids[0]));
    let first = s.take_tab();
    let before = format!("{first:?}");
    assert_ne!(before, format!("{:?}", TabView::default()));
    // Every field of the foreground went out: it shows an empty tab now.
    assert!(s.sidebar.selected.is_none() && s.page.is_none() && s.page_key.is_none());
    assert!(s.editor.is_none() && !s.loading_page && s.issued_request == 0);
    assert!(s.verdict_request == 0 && s.aggregate.stages.is_empty());
    assert_eq!(
        format!("{:?}", s.take_tab()),
        format!("{:?}", TabView::default())
    );
    s.restore_tab(first);
    let again = s.take_tab();
    assert_eq!(format!("{again:?}"), before);
}

#[test]
fn a_connected_session_has_exactly_one_tab_and_it_is_current() {
    let (s, ids) = app(&["a"]);
    let s = opened(s, ids[0]);
    let tabs = s.tabs_of(ids[0]);
    assert_eq!(tabs.len(), 1);
    assert_eq!(s.current_tab_of(ids[0]), Some(tabs[0]));
    assert!(
        s.parked_tabs.is_empty(),
        "the current tab lives in the view"
    );
    assert_eq!(s.status, ConnStatus::Connected);
}

#[test]
fn reconnect_keeps_the_tab_id() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = opened(s, a);
    let tab = s.current_tab_of(a);
    let (s, _) = update(
        s,
        Input::Event(Event::Failed {
            session: a,
            context: "find",
            error: CoreError::Unreachable,
        }),
    );
    assert_eq!(s.status, ConnStatus::Failed);
    let s = opened(s, a);
    assert_eq!(s.tabs_of(a).len(), 1);
    assert_eq!(s.current_tab_of(a), tab);
}

#[test]
fn a_second_session_has_its_own_tab() {
    let (s, ids) = app(&["a", "b"]);
    let s = opened(opened(s, ids[0]), ids[1]);
    assert_eq!(s.tabs_of(ids[0]).len(), 1);
    assert_eq!(s.tabs_of(ids[1]).len(), 1);
    assert_ne!(s.current_tab_of(ids[0]), s.current_tab_of(ids[1]));
}

#[test]
fn disconnect_drops_the_session_s_tabs() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    let (s, extra) = s.tab_opened(a);
    let extra = extra.expect("a tab of an open session");
    assert!(s.parked_tabs.contains_key(&extra));
    let (s, _) = update(s, Input::Disconnect(a));
    assert!(s.tabs_of(a).is_empty());
    assert!(!s.parked_tabs.contains_key(&extra));
    assert_eq!(s.tabs_of(b).len(), 1, "the other session keeps its tab");
}

#[test]
fn opening_a_tab_of_an_unknown_session_does_nothing() {
    let (s, _) = app(&["a"]);
    let (s, tab) = s.tab_opened(Uuid::new_v4());
    assert_eq!(tab, None);
    assert!(s.tabs.order.is_empty());
}

#[test]
fn tab_current_swaps_fields_and_ns() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = busy(opened(s, a));
    let first = s.current_tab_of(a).expect("a tab");
    let (s, second) = s.tab_opened(a);
    let second = second.expect("a second tab");
    assert_eq!(s.current_tab_of(a), Some(first), "opening does not switch");

    let s = s.tab_current(second);
    assert_eq!(s.current_tab_of(a), Some(second));
    assert_eq!(s.selected_ns(), None, "the new tab shows no collection");
    assert!(s.page.is_none() && s.editor.is_none());
    let parked = s.parked_tabs.get(&first).expect("the first tab is parked");
    assert_eq!(parked.ns, Some(users()));
    assert_eq!(parked.page, Some(page()));
    assert_eq!(parked.issued_request, 41);

    let s = s.tab_current(first);
    assert_eq!(s.current_tab_of(a), Some(first));
    assert_eq!(s.selected_ns(), Some(&users()));
    assert_eq!(s.page, Some(page()));
    assert!(s.parked_tabs.contains_key(&second));
}

#[test]
fn tab_current_of_an_unknown_tab_changes_nothing() {
    let (s, ids) = app(&["a"]);
    let s = busy(opened(s, ids[0]));
    let before = format!("{:?}", s.page);
    let s = s.tab_current(TabId(999));
    assert_eq!(format!("{:?}", s.page), before);
    assert_eq!(s.selected_ns(), Some(&users()));
}

#[test]
fn with_tab_on_a_parked_session_s_parked_tab_restores_both() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(s, a);
    let (s, second) = s.tab_opened(a);
    let second = second.expect("a second tab of a");
    let s = opened(s, b);
    // b is active, a is parked with two tabs (the second parked inside `parked_tabs`).
    assert_eq!(s.active, Some(b));
    let first = s.current_tab_of(a).expect("a's current tab");
    let (s, commands) = s.with_tab(second, |s| {
        assert_eq!(s.active, Some(a), "the tab's session is active inside");
        assert_eq!(s.selected_ns(), None);
        let s = AppState {
            find: FindBarState {
                text: curlybrace_core::query::QueryText {
                    filter: "inside".into(),
                    ..Default::default()
                },
                ..FindBarState::default()
            },
            ..s
        };
        (s, Vec::new())
    });
    assert!(commands.is_empty());
    assert_eq!(s.active, Some(b), "the session in view is back");
    assert_eq!(s.current_tab_of(a), Some(first), "a's current tab is back");
    let edited = s
        .parked_tabs
        .get(&second)
        .expect("the second tab is parked");
    assert_eq!(edited.find.text.filter, "inside");
    assert_eq!(
        s.parked[&a].find.text.filter, "",
        "the first tab is untouched"
    );
}

#[test]
fn with_tab_on_an_unknown_tab_changes_nothing() {
    let (s, ids) = app(&["a"]);
    let s = busy(opened(s, ids[0]));
    let (s, commands) = s.with_tab(TabId(999), |_| panic!("must not run"));
    assert!(commands.is_empty());
    assert_eq!(s.page, Some(page()));
}

#[test]
fn with_tab_on_the_current_tab_just_runs() {
    let (s, ids) = app(&["a"]);
    let s = busy(opened(s, ids[0]));
    let tab = s.current_tab_of(ids[0]).expect("a tab");
    let (s, _) = s.with_tab(tab, |s| {
        assert_eq!(s.page, Some(page()));
        (s, Vec::new())
    });
    assert_eq!(s.current_tab_of(ids[0]), Some(tab));
}

#[test]
fn closing_a_parked_tab_forgets_it() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = opened(s, a);
    let (s, second) = s.tab_opened(a);
    let second = second.expect("a tab");
    let s = s.tab_closed(second);
    assert_eq!(s.tabs_of(a).len(), 1);
    assert!(s.parked_tabs.is_empty());
}

#[test]
fn closing_the_current_tab_brings_another_one_in_and_the_last_tab_stays() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = busy(opened(s, a));
    let first = s.current_tab_of(a).expect("a tab");
    let (s, second) = s.tab_opened(a);
    let second = second.expect("a tab");
    let s = s.tab_closed(first);
    assert_eq!(s.current_tab_of(a), Some(second));
    assert_eq!(
        s.selected_ns(),
        None,
        "the surviving tab shows its own view"
    );
    let s = s.tab_closed(second);
    assert_eq!(s.tabs_of(a), vec![second], "a session's only tab stays");
}

#[test]
fn closing_the_current_tab_of_a_session_in_the_background_leaves_the_view_in_front_alone() {
    use crate::app::tabs::test_support::{two_tabs, with_second_session};
    let t = two_tabs();
    let (s, b, _) = with_second_session(t.s);
    assert_eq!(s.active, Some(b));
    let first = s.current_tab_of(t.a).expect("a's current tab");
    let in_front = s.selected_ns().cloned();
    let s = s.tab_closed(first);
    assert_eq!(s.active, Some(b), "the session in front is still in front");
    assert_eq!(
        s.selected_ns().cloned(),
        in_front,
        "its tab was not touched"
    );
    assert_eq!(
        s.current_tab_of(t.a),
        Some(t.second),
        "a's other tab took over"
    );
    assert_eq!(
        s.parked[&t.a].sidebar.selected,
        Some(Namespace::new("app", "orders")),
        "inside a's view"
    );
    assert!(!s.parked_tabs.contains_key(&t.second));
}

#[test]
fn tab_ids_are_not_reused_after_the_state_is_cleared() {
    let (s, ids) = app(&["a"]);
    let s = opened(s, ids[0]);
    let before = s.current_tab_of(ids[0]).expect("a tab");
    let s = s.session_cleared();
    assert!(s.tabs.order.is_empty());
    let s = opened(s, ids[0]);
    assert_ne!(
        s.current_tab_of(ids[0]),
        Some(before),
        "the number is not handed out twice"
    );
}

#[test]
fn a_cleared_state_has_no_tabs() {
    let (s, ids) = app(&["a"]);
    let s = opened(s, ids[0]);
    let (s, _) = s.tab_opened(ids[0]);
    let s = s.session_cleared();
    assert!(s.tabs.order.is_empty() && s.parked_tabs.is_empty());
}

#[test]
fn opening_a_collection_goes_to_the_current_tab() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = opened(s, a);
    let (s, second) = s.tab_opened(a);
    let second = second.expect("a tab");
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(users())));
    assert_eq!(s.selected_ns(), Some(&users()), "the current tab opened it");
    assert_eq!(s.parked_tabs[&second].ns, None);
    let _ = Pending {
        ns: users(),
        editor: None,
        tab: None,
    };
}
