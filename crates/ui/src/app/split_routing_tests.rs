//! Split view through the reducer: where a pane's input goes, focus and the shown step (#159,
//! slice 3). The fixtures are shared with `split_actions_tests`.
use super::super::tab_actions::TabAction;
use super::tests::{answered, finds_in, opened, split_of, tabs};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::tabs::Freshness;
use crate::app::tabs::test_support::{ev, orders, parked, users, with_second_session};
use crate::viewmodel::{CollectionTab, SidebarAction};
use curlybrace_core::runtime::Event;
use curlybrace_core::store::Namespace;

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
fn for_tab_input_applies_in_its_tab_and_leaves_the_focus() {
    // A pane that acts alone (an automatic refresh) lands in its own tab and takes nothing.
    let (s, a, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let input = Input::ForTab {
        tab: second,
        input: Box::new(Input::Tab(CollectionTab::Indexes)),
    };
    let (s, commands) = update(s, input);
    assert_eq!(s.active_tab(), Some(first), "the focus did not move");
    assert_eq!(
        parked(&s, second).tab,
        CollectionTab::Indexes,
        "it landed in its tab"
    );
    assert_eq!(s.tab, CollectionTab::Documents);
    assert!(
        commands.iter().all(|c| c.session() == Some(a)),
        "{commands:?}"
    );
    let split = split_of(&s);
    assert_eq!((split.primary, split.secondary), (first, second));
}

#[test]
fn for_tab_input_on_another_connection_runs_in_that_connection() {
    let (s, a, first, _) = answered();
    let (s, b, _) = with_second_session(s);
    let (s, _) = tabs(s, TabAction::OpenInOtherPane(first));
    let before = s.active_tab();
    let input = Input::ForTab {
        tab: first,
        input: Box::new(Input::Tab(CollectionTab::Indexes)),
    };
    let (s, commands) = update(s, input);
    assert_eq!(s.active, Some(b), "the foreground connection is the same");
    assert_eq!(s.active_tab(), before);
    assert!(!commands.is_empty());
    assert!(
        commands.iter().all(|c| c.session() == Some(a)),
        "{commands:?}"
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
fn a_new_tab_in_a_split_is_not_closed_when_focus_moves_to_the_other_pane() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::New);
    let fresh = s.active_tab().expect("the new tab");
    assert!(fresh != first && fresh != second);
    assert_eq!(
        split_of(&s).primary,
        fresh,
        "the new tab is the active pane"
    );
    // Clicking the other pane swaps the roles: the empty tab is the partner now.
    let (s, _) = tabs(s, TabAction::FocusOtherPane);
    let split = split_of(&s);
    assert_eq!(
        split.secondary, fresh,
        "still a split, with the empty tab as a pane"
    );
    assert!(
        s.tabs.owner.contains_key(&fresh),
        "the empty tab was not closed"
    );
    // Leaving it through the strip (another tab takes the active pane) keeps it as the partner;
    // picking a collection in it later fills the pane.
    let (s, _) = tabs(s, TabAction::FocusOtherPane);
    assert_eq!(split_of(&s).primary, fresh);
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

#[test]
fn an_emptied_partner_is_still_a_visible_tab() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = s.with_tab(second, |s| (s.current_tab_emptied(), Vec::new()));
    let (s, _) = tabs(s, TabAction::Activate(first));
    assert!(
        s.visible_tabs().contains(&second),
        "the strip shows the partner"
    );
    assert!(s.visible_tabs().len() >= 2, "so the strip is drawn");
    // The limit counts it: the order never holds more than the limit.
    assert_eq!(s.visible_tabs().len(), s.tabs.order.len());
}

#[test]
fn a_new_tab_made_the_partner_stays_in_the_strip_and_closes_with_the_others() {
    let (s, _, first, second) = answered();
    let (s, _) = tabs(s, TabAction::ToggleSplit);
    let (s, _) = tabs(s, TabAction::New);
    let fresh = s.active_tab().expect("the new tab");
    let (s, _) = tabs(s, TabAction::FocusOtherPane);
    let (s, _) = tabs(s, TabAction::Close(first));
    assert_eq!(
        split_of(&s).secondary,
        fresh,
        "the empty tab is the partner"
    );
    assert!(s.visible_tabs().contains(&fresh) && s.visible_tabs().len() >= 2);
    let (s, _) = tabs(s, TabAction::CloseOthers(second));
    assert!(!s.tabs.owner.contains_key(&fresh), "Close others closed it");
    assert!(s.tabs.split.is_none(), "and the split ended with it");
}
