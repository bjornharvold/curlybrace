//! Bounds on the pages parked tabs retain (#159, T8): by count, by bytes, and never the tab being
//! looked at. An evicted tab is stale and reads its page again when shown.
use super::{RETAINED_PARKED, evict_within};
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tab_actions::TabAction;
use crate::app::tabs::test_support::{ev, find_request, orders, parked, results, two_tabs, users};
use crate::app::tabs::{Freshness, TabId};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// `n` tabs of one session in all, each holding a page; the newest opened is the one shown. The
/// first tab is the oldest, the second (never shown since it was opened) the least recently used.
fn with_pages(n: usize) -> (AppState, Uuid, Vec<TabId>) {
    let t = two_tabs();
    let a = t.a;
    let s = ev(t.s, results(a, users(), t.first_find, 1));
    let mut s = ev(s, results(a, orders(), t.second_find, 1));
    let mut tabs = vec![s.tabs.order[0], t.second];
    for i in 0..n.saturating_sub(2) {
        let ns = Namespace::new("app", format!("c{i}"));
        let (next, commands) = update(
            s,
            Input::Tabs(TabAction::OpenInNew {
                session: a,
                ns: ns.clone(),
            }),
        );
        let request = find_request(&commands);
        tabs.push(next.active_tab().expect("the new tab"));
        s = ev(next, results(a, ns, request, 1));
    }
    (s, a, tabs)
}

fn has_page(s: &AppState, tab: TabId) -> bool {
    parked(s, tab).page.is_some()
}

#[test]
fn the_seventh_parked_page_is_evicted_oldest_first() {
    // Seven parked tabs and the one shown.
    let (s, _, tabs) = with_pages(RETAINED_PARKED + 2);
    let parked_ids: Vec<TabId> = tabs[..tabs.len() - 1].to_vec();
    assert_eq!(parked_ids.len(), RETAINED_PARKED + 1);
    let evicted: Vec<TabId> = parked_ids
        .iter()
        .copied()
        .filter(|t| !has_page(&s, *t))
        .collect();
    assert_eq!(
        evicted,
        vec![tabs[1]],
        "only the least recently used one: {evicted:?}"
    );
    assert_eq!(parked(&s, tabs[1]).freshness, Freshness::Stale);
    assert!(s.page.is_some(), "the tab being looked at keeps its page");
}

#[test]
fn six_parked_pages_are_all_kept() {
    let (s, _, tabs) = with_pages(RETAINED_PARKED + 1);
    assert!(tabs[..tabs.len() - 1].iter().all(|t| has_page(&s, *t)));
}

#[test]
fn byte_budget_evicts_before_count() {
    let (mut s, _, tabs) = with_pages(5);
    let each = parked(&s, tabs[0]).page_key.map_or(0, |k| k.bytes);
    assert!(each > 0);
    // Four parked pages, room for two by bytes (and six by count).
    evict_within(&mut s, RETAINED_PARKED, 2 * each);
    let kept: Vec<TabId> = tabs[..4]
        .iter()
        .copied()
        .filter(|t| has_page(&s, *t))
        .collect();
    assert_eq!(kept.len(), 2);
    // The two most recently shown parked tabs are the ones kept.
    assert!(
        kept.contains(&tabs[3]) && kept.contains(&tabs[2]),
        "{kept:?}"
    );
}

#[test]
fn active_and_other_sessions_current_pages_are_never_evicted() {
    let (mut s, _, tabs) = with_pages(4);
    evict_within(&mut s, 0, 0);
    assert!(
        tabs[..3].iter().all(|t| !has_page(&s, *t)),
        "every parked page went"
    );
    assert!(s.page.is_some(), "the shown tab's page stays");
    assert!(s.page_key.is_some());
}

#[test]
fn an_evicted_tab_is_stale_and_refetches_when_shown() {
    let (s, _, tabs) = with_pages(RETAINED_PARKED + 2);
    let evicted = tabs[1];
    let (s, commands) = update(s, Input::Tabs(TabAction::Activate(evicted)));
    let finds: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::Find { ns, query, .. } => Some((ns.clone(), query.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(finds.len(), 1, "{commands:?}");
    assert_eq!(finds[0].0, orders());
    assert_eq!(finds[0].1, s.page_query);
    assert!(s.loading_page || s.page.is_none());
}
