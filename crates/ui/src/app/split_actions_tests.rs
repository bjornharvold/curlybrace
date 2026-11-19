//! Split view through the reducer (#159, slice 3): no UI. The panes are the active tab and one
//! partner; every path that moves, closes or loses a tab is walked against the split so it never
//! names a tab that is gone.
use super::super::tab_actions::{MAX_TABS, TabAction};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::test_support::{
    ev, find_request, orders, parked, results, two_tabs, users, with_second_session,
};
use crate::app::tabs::{Freshness, TabId};
use crate::viewmodel::split_layout::Orientation;
use crate::viewmodel::{CollectionTab, SidebarAction};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn tabs(s: AppState, action: TabAction) -> (AppState, Vec<Command>) {
    update(s, Input::Tabs(action))
}

fn finds_in(commands: &[Command]) -> Vec<(Uuid, Namespace)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::Find { session, ns, .. } => Some((*session, ns.clone())),
            _ => None,
        })
        .collect()
}

/// Two tabs of one session, both answered: tab one (shown) on `app.users`, tab two (parked) on
/// `app.orders`.
fn answered() -> (AppState, Uuid, TabId, TabId) {
    let t = two_tabs();
    let s = ev(t.s, results(t.a, users(), t.first_find, 2));
    let s = ev(s, results(t.a, orders(), t.second_find, 2));
    let first = s.active_tab().expect("a shown tab");
    (s, t.a, first, t.second)
}

fn split_of(s: &AppState) -> crate::app::tab_strip::Split {
    s.tabs.split.clone().expect("a split")
}

/// Opens `ns` in a new tab of `a` and answers its find. The new tab is the one shown.
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

#[test]
fn toggle_split_pairs_with_the_mru_other_tab() {
    let (s, a, first, second) = answered();
    let (s, third) = opened(s, a, Namespace::new("app", "events"));
    // Recency: third, then (by looking) second, then first.
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (s, _) = tabs(s, TabAction::Activate(second));
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let split = split_of(&s);
    assert_eq!(split.primary, first);
    assert_eq!(
        split.secondary, second,
        "the one used just before, not {third:?}"
    );
    assert_eq!(
        s.active_tab(),
        Some(first),
        "the split changes nothing the person looks at"
    );
    assert_eq!(split.orientation, Orientation::SideBySide);
    assert_eq!(split.ratio, 0.5);
}

#[test]
fn toggle_again_closes_the_split() {
    let (s, _, _, _) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    assert!(s.tabs.split.is_some());
    let (s, commands) = tabs(s, TabAction::ToggleSplit);
    assert!(s.tabs.split.is_none());
    assert!(commands.is_empty());
}

#[test]
fn closing_a_split_tab_ends_the_split() {
    // The partner closes.
    let (s, _, _, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::Close(second));
    assert!(s.tabs.split.is_none(), "the partner closed");
    // The shown tab closes (its neighbour is shown instead).
    let (s, _, first, _) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::Close(first));
    assert!(s.tabs.split.is_none(), "the shown tab closed");
}

#[test]
fn a_tab_outside_the_split_closing_leaves_it() {
    let (s, a, first, second) = answered();
    let (s, third) = opened(s, a, Namespace::new("app", "events"));
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(second));
    let (s, _) = tabs(s, TabAction::Close(third));
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (first, second));
}

#[test]
fn in_tab_input_focuses_that_tab_first() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    assert_eq!(s.active_tab(), Some(first));
    let input = Input::InTab {
        tab: second,
        input: Box::new(Input::Tab(CollectionTab::Indexes)),
    };
    let (s, _) = update(s, input);
    assert_eq!(
        s.active_tab(),
        Some(second),
        "the pane's input made it the active tab"
    );
    assert_eq!(s.tab, CollectionTab::Indexes, "and landed in it");
    assert_eq!(
        parked(&s, first).tab,
        CollectionTab::Documents,
        "not in the other pane"
    );
}

#[test]
fn in_tab_input_for_a_closed_tab_is_dropped() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::Close(second));
    let input = Input::InTab {
        tab: second,
        input: Box::new(Input::Tab(CollectionTab::Indexes)),
    };
    let (s, commands) = update(s, input);
    assert!(commands.is_empty());
    assert_eq!(s.active_tab(), Some(first));
    assert_eq!(s.tab, CollectionTab::Documents);
}

#[test]
fn focus_other_pane_swaps_roles_not_positions() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let before = split_of(&s);
    assert!(
        !before.secondary_first,
        "the shown tab starts in the first slot"
    );
    let (s, _) = tabs(s, TabAction::FocusOtherPane);
    assert_eq!(s.active_tab(), Some(second));
    let after = split_of(&s);
    assert_eq!((after.primary, after.secondary), (second, first));
    // The tab that was in the first slot is still there: it is the partner now.
    assert!(after.secondary_first, "the old tab did not move on screen");
    let (s, _) = tabs(s, TabAction::FocusOtherPane);
    let back = split_of(&s);
    assert_eq!(
        (back.primary, back.secondary, back.secondary_first),
        (first, second, false)
    );
}

#[test]
fn show_other_pane_is_the_same_swap() {
    let (s, _, _, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::ShowOtherPane);
    assert_eq!(s.active_tab(), Some(second));
}

#[test]
fn opening_in_other_pane_with_one_tab_duplicates_it() {
    let t = two_tabs();
    let s = ev(t.s, results(t.a, users(), t.first_find, 2));
    // Close tab two: one tab on `app.users` is left.
    let (s, _) = tabs(s, TabAction::Close(t.second));
    let only = s.active_tab().expect("a tab");
    assert_eq!(s.tabs_of(t.a), vec![only]);
    let (s, commands) = tabs(s, TabAction::OpenInOtherPane(only));
    assert_eq!(s.tabs_of(t.a).len(), 2, "a copy was opened");
    let split = split_of(&s);
    assert!([split.primary, split.secondary].contains(&only));
    assert_eq!(s.tab_ns(split.primary), Some(users()));
    assert_eq!(s.tab_ns(split.secondary), Some(users()));
    assert_eq!(finds_in(&commands).len(), 1, "the copy runs the query once");
    assert!(split.secondary_first, "the original stays where it was");
    assert_eq!(split.secondary, only);
}

#[test]
fn duplicate_does_not_copy_the_split() {
    let (s, _, first, _) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::Duplicate(first));
    let copy = s.active_tab().expect("the copy");
    let split = s.tabs.split.clone();
    // The split is still the two panes it was; the copy is a plain tab of the strip.
    assert!(split.is_none_or(|s| s.primary != s.secondary && s.secondary != copy));
}

#[test]
fn open_in_other_pane_at_the_limit_opens_nothing() {
    let (mut s, a, first, _) = answered();
    while s.visible_tabs().len() < MAX_TABS {
        let n = s.tabs.order.len();
        s = opened(s, a, Namespace::new("app", format!("c{n}"))).0;
    }
    let (s, _) = tabs(s, TabAction::Activate(first));
    let before = s.tabs.order.len();
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(first));
    assert_eq!(s.tabs.order.len(), before);
    assert!(s.tabs.split.is_none());
    assert!(s.tabs.notice.is_some(), "the limit notice says why");
}

#[test]
fn a_stale_partner_is_read_when_the_split_opens() {
    let (s, a, _, second) = answered();
    let s = ev(
        s,
        Event::Mutated {
            session: a,
            ns: orders(),
        },
    );
    assert_eq!(parked(&s, second).freshness, Freshness::Stale);
    let (s, commands) = tabs(s, TabAction::ToggleSplit);
    assert_eq!(
        finds_in(&commands),
        vec![(a, orders())],
        "the one shared step ran for the pane"
    );
    assert_eq!(s.freshness_of(second), Freshness::Fresh);
    assert_eq!(
        parked(&s, second).freshness,
        Freshness::Fresh,
        "and the mark stayed cleared"
    );
    // Showing the split again does not read again.
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (_, commands) = tabs(s, TabAction::ToggleSplit);
    assert!(finds_in(&commands).is_empty());
}

#[test]
fn an_evicted_partner_reads_its_page_when_the_split_opens() {
    let (mut s, a, _, second) = answered();
    if let Some(view) = s.parked_tabs.get_mut(&second) {
        view.page = None;
        view.page_key = None;
        view.freshness = Freshness::Stale;
    }
    let (_, commands) = tabs(s, TabAction::ToggleSplit);
    assert_eq!(finds_in(&commands), vec![(a, orders())]);
}

#[test]
fn a_dormant_partner_sends_nothing() {
    let (mut s, _, _, second) = answered();
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([second]);
    let (s, commands) = tabs(s, TabAction::ToggleSplit);
    assert!(
        commands.is_empty(),
        "a dormant pane shows its Run: {commands:?}"
    );
    assert_eq!(s.freshness_of(second), Freshness::Dormant);
}

#[test]
fn a_pane_on_another_connection_activates_that_session_when_focused() {
    let (s, a, first, _) = answered();
    let (s, b, _) = with_second_session(s);
    assert_eq!(s.active, Some(b));
    let b_tab = s.active_tab().expect("b's shown tab");
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(first));
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (b_tab, first));
    // The pane's input is applied in the session that owns the tab.
    let input = Input::InTab {
        tab: first,
        input: Box::new(Input::Tab(CollectionTab::Indexes)),
    };
    let (s, commands) = update(s, input);
    assert_eq!(s.active, Some(a));
    assert_eq!(s.active_tab(), Some(first));
    assert!(
        commands.iter().all(|c| c.session() == Some(a)),
        "every command is for the pane's own connection: {commands:?}"
    );
    assert!(
        !commands.is_empty(),
        "the Indexes list was asked for: {commands:?}"
    );
    let after = split_of(&s);
    assert_eq!((after.primary, after.secondary), (first, b_tab));
}

#[test]
fn a_stale_partner_on_another_connection_is_read_in_its_own_session() {
    let (s, a, _, second) = answered();
    let (s, b, _) = with_second_session(s);
    let s = ev(
        s,
        Event::Mutated {
            session: a,
            ns: orders(),
        },
    );
    // `second` is a parked tab of the background session `a`, marked stale by the write.
    assert_eq!(s.freshness_of(second), Freshness::Stale);
    let (s, commands) = tabs(s, TabAction::OpenInOtherPane(second));
    assert_eq!(
        finds_in(&commands),
        vec![(a, orders())],
        "sent for a, not for {b:?}"
    );
    assert_eq!(s.active, Some(b), "the foreground session did not change");
    assert_eq!(s.freshness_of(second), Freshness::Fresh);
    assert_eq!(parked(&s, second).freshness, Freshness::Fresh);
}

#[test]
fn run_dialog_closes_when_focus_moves_to_the_other_pane() {
    let (s, _, first, _) = answered();
    let (s, _) = update(s, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::AddStage("$out".into())),
    );
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::Edit(1, r#"{ "$out": "dest" }"#.into())),
    );
    let (s, _) = update(s, Input::Aggregate(AggregateAction::RunClicked));
    assert!(s.aggregate.run.dialog.is_some(), "the confirmation is open");
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    assert!(
        s.aggregate.run.dialog.is_some(),
        "splitting does not close it"
    );
    let (s, _) = tabs(s, TabAction::FocusOtherPane);
    assert!(
        parked(&s, first).aggregate.run.dialog.is_none(),
        "the pane that lost focus has no confirmation open"
    );
}

#[test]
fn split_partner_is_not_evicted() {
    let (s, a, _, second) = answered();
    let (mut s, _) = tabs(s, TabAction::ToggleSplit);
    for i in 0..9 {
        s = opened(s, a, Namespace::new("app", format!("c{i}"))).0;
        // Each new tab is the shown one; the split keeps its partner.
        let split = split_of(&s);
        assert_eq!(split.secondary, second);
    }
    assert!(
        parked(&s, second).page.is_some(),
        "the partner kept its page"
    );
    assert_eq!(s.freshness_of(second), Freshness::Fresh);
    // Without the split the same tab is the first to go.
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let oldest = s.tabs.order[0];
    let (s, _) = tabs(s, TabAction::Activate(oldest));
    assert!(
        parked(&s, second).page.is_none(),
        "evicted once it is no longer a partner"
    );
}

#[test]
fn activating_a_third_tab_keeps_the_partner() {
    let (s, a, first, second) = answered();
    let (s, third) = opened(s, a, Namespace::new("app", "events"));
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(second));
    let (s, _) = tabs(s, TabAction::Activate(third));
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (third, second));
    // Activating the partner swaps the roles.
    let (s, _) = tabs(s, TabAction::Activate(second));
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (second, third));
}

#[test]
fn closing_the_other_panes_connection_ends_the_split() {
    let (s, a, first, _) = answered();
    let (s, b, _) = with_second_session(s);
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(first));
    assert!(s.tabs.split.is_some());
    let (s, _) = update(s, Input::Disconnect(a));
    assert!(
        s.tabs.split.is_none(),
        "the partner went with its connection"
    );
    assert_eq!(s.active, Some(b));
}

#[test]
fn closing_the_shown_panes_connection_ends_the_split() {
    let (s, a, first, _) = answered();
    let (s, b, _) = with_second_session(s);
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(first));
    let (s, _) = update(s, Input::Disconnect(b));
    assert!(s.tabs.split.is_none());
    assert!(s.tabs.owner.values().all(|session| *session == a));
}

#[test]
fn a_lost_connection_keeps_the_split_with_dormant_panes() {
    let (s, a, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = update(
        s,
        Input::Event(Event::Failed {
            session: a,
            context: "list_databases",
            error: curlybrace_core::error::CoreError::Unreachable,
        }),
    );
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (first, second));
    assert_eq!(s.freshness_of(second), Freshness::Dormant);
}

#[test]
fn a_cleared_runtime_ends_the_split() {
    let (s, _, _, _) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let s = s.session_cleared();
    assert!(s.tabs.split.is_none());
}

#[test]
fn reordering_does_not_touch_the_split() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::Move { tab: second, to: 0 });
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (first, second));
}

#[test]
fn close_others_and_right_leave_a_partner_on_another_connection() {
    let (s, a, first, _) = answered();
    let (s, b, _) = with_second_session(s);
    let b_tab = s.active_tab().expect("b's tab");
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(first));
    // Close the other tabs of b: a's tabs are not b's to close.
    let (s, _) = tabs(s, TabAction::CloseOthers(b_tab));
    assert!(
        s.tabs.owner.contains_key(&first),
        "{a:?}'s tab is still open"
    );
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (b_tab, first));
    let (s, _) = tabs(s, TabAction::CloseRight(b_tab));
    assert_eq!(split_of(&s).secondary, first);
    assert_eq!(s.active, Some(b));
}

#[test]
fn close_others_in_the_partners_connection_ends_the_split() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(second));
    let (s, _) = tabs(s, TabAction::CloseOthers(first));
    assert!(
        s.tabs.split.is_none(),
        "the partner was one of the closed tabs"
    );
}

#[test]
fn reopening_a_closed_tab_does_not_restore_a_split() {
    let (s, _, _, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::Close(second));
    let (s, _) = tabs(s, TabAction::ReopenClosed);
    assert!(s.tabs.split.is_none());
}

#[test]
fn an_emptied_partner_ends_the_split() {
    let (s, _, _, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    // The partner blanks (its collection was dropped): a pane with nothing in it is no pane.
    let (s, _) = s.with_tab(second, |s| (s.current_tab_emptied(), Vec::new()));
    let oldest = s.tabs.order[0];
    let (s, _) = tabs(s, TabAction::Activate(oldest));
    assert!(s.tabs.split.is_none(), "an empty partner ends the split");
}

#[test]
fn ratio_and_orientation_are_set_and_clamped() {
    let (s, _, _, _) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::SetRatio(0.9));
    assert_eq!(split_of(&s).ratio, 0.75);
    let (s, _) = tabs(s, TabAction::SetRatio(0.1));
    assert_eq!(split_of(&s).ratio, 0.25);
    let (s, _) = tabs(s, TabAction::SetOrientation(Orientation::Stacked));
    assert_eq!(split_of(&s).orientation, Orientation::Stacked);
    // Without a split they change nothing.
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::SetRatio(0.6));
    assert!(s.tabs.split.is_none());
}

#[test]
fn a_shown_pane_that_is_stale_is_read_by_the_pane_shown_step() {
    let (s, a, _, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let s = ev(
        s,
        Event::Mutated {
            session: a,
            ns: orders(),
        },
    );
    assert_eq!(parked(&s, second).freshness, Freshness::Stale);
    let (s, commands) = tabs(s, TabAction::PaneShown(second));
    assert_eq!(finds_in(&commands), vec![(a, orders())]);
    let (_, again) = tabs(s, TabAction::PaneShown(second));
    assert!(again.is_empty(), "once");
}

#[test]
fn nothing_is_sent_for_a_pane_that_is_not_in_the_split() {
    let (s, _, _, second) = answered();
    let (s, commands) = tabs(s, TabAction::PaneShown(second));
    assert!(commands.is_empty());
    assert!(s.tabs.split.is_none());
    // `SidebarAction` is unused by the split: it still opens in the shown tab.
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(users())));
    assert!(s.tabs.split.is_none());
}
