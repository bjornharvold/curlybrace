use super::{TabLabel, labels};
use crate::I18n;
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tab_actions::TabAction;
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{ev, orders, two_tabs, users, with_second_session};
use crate::viewmodel::{DocAction, SidebarAction};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn all(s: &AppState) -> Vec<(TabId, TabLabel)> {
    labels(s, &I18n::english())
}

fn open_in_new(s: AppState, a: Uuid, ns: Namespace) -> AppState {
    update(s, Input::Tabs(TabAction::OpenInNew { session: a, ns })).0
}

#[test]
fn title_is_the_collection() {
    let t = two_tabs();
    let titles: Vec<String> = all(&t.s).into_iter().map(|(_, l)| l.title).collect();
    assert_eq!(titles, vec!["users", "orders"]);
}

#[test]
fn same_collection_in_two_databases_shows_db_dot_collection() {
    let t = two_tabs();
    let s = open_in_new(t.s, t.a, Namespace::new("other", "users"));
    let titles: Vec<String> = all(&s).into_iter().map(|(_, l)| l.title).collect();
    assert!(titles.contains(&"app.users".to_string()), "{titles:?}");
    assert!(titles.contains(&"other.users".to_string()), "{titles:?}");
    assert!(
        titles.contains(&"orders".to_string()),
        "only the clashing ones: {titles:?}"
    );
}

#[test]
fn still_ambiguous_adds_the_connection() {
    let t = two_tabs();
    let (s, _, _) = with_second_session(t.s);
    // Both connections now show app.users and app.orders: the same names, on two connections.
    let titles: Vec<String> = all(&s).into_iter().map(|(_, l)| l.title).collect();
    assert!(titles.contains(&"users · a".to_string()), "{titles:?}");
    assert!(titles.contains(&"users · b".to_string()), "{titles:?}");
    assert!(titles.contains(&"orders · a".to_string()), "{titles:?}");
}

#[test]
fn duplicates_share_a_title_and_differ_by_position() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Tabs(TabAction::Duplicate(t.second)));
    let labels = all(&s);
    let dup: Vec<&TabLabel> = labels
        .iter()
        .map(|(_, l)| l)
        .filter(|l| l.title == "orders")
        .collect();
    assert_eq!(dup.len(), 2);
    assert_eq!(dup[0].title, dup[1].title);
    assert_ne!(
        dup[0].accessible, dup[1].accessible,
        "told apart by position"
    );
}

#[test]
fn empty_tab_title() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Tabs(TabAction::New));
    let labels = all(&s);
    let empty = labels
        .last()
        .map(|(_, l)| l.title.clone())
        .or_else(|| labels.first().map(|(_, l)| l.title.clone()));
    assert!(
        labels.iter().any(|(_, l)| l.title == "Select a collection"),
        "{empty:?}"
    );
}

#[test]
fn only_the_active_empty_tab_is_shown() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Tabs(TabAction::New));
    let (s, _) = update(s, Input::Tabs(TabAction::New));
    let shown = all(&s).len();
    assert_eq!(shown, 3, "two collections and the active empty tab");
}

#[test]
fn accessible_name_names_production_and_read_only() {
    let t = two_tabs();
    let mut s = t.s;
    s.production = true;
    s.read_only = true;
    let labels = all(&s);
    let first = &labels[0].1;
    assert!(
        first.accessible.contains("production"),
        "{}",
        first.accessible
    );
    assert!(
        first.accessible.contains("read-only"),
        "{}",
        first.accessible
    );
    assert!(
        first.accessible.contains("tab 1 of 2"),
        "{}",
        first.accessible
    );
    assert!(first.badge.production && first.badge.read_only);
}

#[test]
fn tooltip_names_the_sub_tab() {
    let t = two_tabs();
    let labels = all(&t.s);
    assert!(
        labels[0].1.tooltip.contains("app.users"),
        "{}",
        labels[0].1.tooltip
    );
    assert!(
        labels[0].1.tooltip.contains("Documents"),
        "{}",
        labels[0].1.tooltip
    );
    assert!(labels[0].1.tooltip.contains('a'), "names the connection");
}

#[test]
fn a_tab_with_an_open_editor_says_it_has_unsaved_changes() {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Doc(DocAction::Insert));
    let labels = all(&s);
    assert!(labels[0].1.unsaved);
    assert!(
        labels[0].1.accessible.contains("unsaved"),
        "{}",
        labels[0].1.accessible
    );
    assert!(!labels[1].1.unsaved);
    assert!(!labels[1].1.accessible.contains("unsaved"));
}

#[test]
fn a_dormant_tab_is_labelled_dormant() {
    let t = two_tabs();
    let mut s = t.s;
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([t.second]);
    let labels = all(&s);
    assert!(labels[1].1.dormant && !labels[0].1.dormant);
}

#[allow(dead_code)]
fn unused(s: AppState) -> AppState {
    let _ = (users(), orders(), SidebarAction::Refresh);
    ev(
        s,
        curlybrace_core::runtime::Event::Connected {
            session: Uuid::nil(),
            epoch: 0,
            topology: None,
        },
    )
}
