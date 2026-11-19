//! A dormant tab sends nothing but through its Run (#159, review fixes): not after a write from
//! another tab, not after an index write, and Run clears the stale mark it wakes.
use super::TabAction;
use super::tests::{finds, opened, tabs};
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::test_support::{two_tabs, users};
use crate::app::tabs::{Freshness, TabId};
use crate::viewmodel::{DocAction, EditorAction};
use curlybrace_core::runtime::{Command, Event};
use uuid::Uuid;

/// Two tabs on `app.users`: the first shown and dormant, the second (parked) saving a document.
fn dormant_shown_and_a_write_from_the_other() -> (AppState, TabId, Uuid) {
    let t = two_tabs();
    let first = t.s.active_tab().expect("first");
    let (s, _writer) = opened(t.s, t.a, users());
    let (s, _) = update(s, Input::Doc(DocAction::Insert));
    let text = s
        .editor
        .as_ref()
        .map(|e| e.text.clone())
        .expect("an editor");
    let (s, saved) = update(
        s,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    assert!(matches!(saved.as_slice(), [Command::Insert { .. }]));
    let (mut s, _) = tabs(s, TabAction::Activate(first));
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([first]);
    (s, first, t.a)
}

#[test]
fn a_write_from_another_tab_does_not_wake_a_dormant_tab() {
    let (s, first, a) = dormant_shown_and_a_write_from_the_other();
    let mutated = Event::Mutated {
        session: a,
        ns: users(),
    };
    let (s, commands) = update(s, Input::Event(mutated));
    assert!(
        finds(&commands).is_empty(),
        "no read for a dormant tab: {commands:?}"
    );
    assert_eq!(
        s.freshness_of(first),
        Freshness::Dormant,
        "still waiting for Run"
    );
    assert!(s.tabs.stale.contains(&first), "and known to be out of date");
}

#[test]
fn an_index_write_does_not_make_a_dormant_tab_list_its_indexes() {
    let (s, first, a) = dormant_shown_and_a_write_from_the_other();
    let changed = Event::IndexChanged {
        session: a,
        ns: users(),
    };
    let (s, commands) = update(s, Input::Event(changed));
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, Command::ListIndexes { .. })),
        "{commands:?}"
    );
    assert_eq!(s.freshness_of(first), Freshness::Dormant);
}

#[test]
fn a_dormant_aggregate_tab_waits_for_an_edit_or_refresh_before_its_first_preview() {
    use crate::app::AggregateAction;
    use crate::app::tabs::test_support::{act, ev, parked};
    use crate::viewmodel::CollectionTab;
    use curlybrace_core::error::CoreError;
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let s = ev(
        s,
        Event::Failed {
            session: t.a,
            context: "find",
            error: CoreError::Unreachable,
        },
    );
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
    let parked_view = parked(&s, t.second);
    assert!(
        !parked_view
            .aggregate
            .wants_auto_refresh(parked_view.aggregate_view.auto_sent),
        "a parked dormant draft"
    );
    let (s, commands) = tabs(s, TabAction::Activate(t.second));
    assert!(commands.is_empty());
    assert!(
        !s.aggregate.wants_auto_refresh(s.aggregate_view.auto_sent),
        "the draw would send a preview by itself"
    );
    // Its first edit asks again.
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::AddStage("$limit".into())),
    );
    assert!(s.aggregate.in_flight.is_some());
}

#[test]
fn run_clears_the_stale_mark_it_wakes() {
    // A parked tab that is dormant and was marked stale by a write, then run.
    let t = two_tabs();
    let mut s = t.s;
    s.tabs = std::mem::take(&mut s.tabs).made_dormant([t.second]);
    let mutated = Event::Mutated {
        session: t.a,
        ns: crate::app::tabs::test_support::orders(),
    };
    let (mut s, _) = update(s, Input::Event(mutated));
    assert_eq!(
        crate::app::tabs::test_support::parked(&s, t.second).freshness,
        Freshness::Stale
    );
    s.explain.guard = false;
    let (s, commands) = tabs(s, TabAction::Run(t.second));
    assert_eq!(finds(&commands).len(), 1);
    assert!(!s.tabs.stale.contains(&t.second), "its read was just sent");
    assert_eq!(s.freshness_of(t.second), Freshness::Fresh);
    // Showing it again re-reads nothing.
    let first = s.tabs.order[0];
    let (s, _) = tabs(s, TabAction::Activate(first));
    let (_, commands) = tabs(s, TabAction::Activate(t.second));
    assert!(finds(&commands).is_empty(), "{commands:?}");
}
