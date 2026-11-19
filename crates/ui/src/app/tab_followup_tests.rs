//! Follow-ups of the review of PR #187 (#159): the profiler read goes to the tab waiting for that
//! database (the current tab first), the numbers a tab awaits are defined once, and closing or
//! switching tabs follows the strip.
use super::{Numbers, Read, owner_of, read_of, read_of_failure};
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{act, ev, parked, two_tabs, users};
use crate::viewmodel::queries::InFlight;
use crate::viewmodel::{CollectionTab, IndexesAction, QueriesAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::ProfilingState;
use uuid::Uuid;

// The profiler read: current tab C (users) and parked tab P (orders) both read `app`.

fn both_reading() -> (AppState, Uuid, TabId) {
    let t = two_tabs();
    let (s, _) = update(t.s, Input::Tab(CollectionTab::Indexes));
    let (s, c1) = update(
        s,
        Input::Indexes(IndexesAction::Queries(QueriesAction::Open)),
    );
    let (s, _) = act(s, t.second, Input::Tab(CollectionTab::Indexes));
    let (s, c2) = act(
        s,
        t.second,
        Input::Indexes(IndexesAction::Queries(QueriesAction::Open)),
    );
    for commands in [&c1, &c2] {
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::ProfilingState { .. }))
        );
    }
    assert!(matches!(
        s.indexes.queries.in_flight,
        Some(InFlight::Read(_))
    ));
    (s, t.a, t.second)
}

fn level(session: Uuid, db: &str) -> Event {
    Event::ProfilingState {
        session,
        db: db.into(),
        outcome: Ok(ProfilingState {
            level: 0,
            slow_ms: 100,
        }),
    }
}

fn reading(s: &AppState, tab: TabId) -> bool {
    matches!(
        parked(s, tab).indexes.queries.in_flight,
        Some(InFlight::Read(_))
    )
}

#[test]
fn the_current_tab_claims_the_first_profiling_answer_it_waits_for() {
    let (s, a, parked_tab) = both_reading();
    let s = ev(s, level(a, "app"));
    assert!(
        s.indexes.queries.in_flight.is_none(),
        "the current tab's read is settled"
    );
    assert!(
        reading(&s, parked_tab),
        "the parked tab still waits for its own"
    );
    let s = ev(s, level(a, "app"));
    assert!(!reading(&s, parked_tab), "the second answer is its");
}

#[test]
fn the_current_tab_claims_the_failure_of_its_own_profiling_read() {
    let (s, a, parked_tab) = both_reading();
    let s = ev(
        s,
        Event::Failed {
            session: a,
            context: "profiling_state",
            error: CoreError::Database("no".into()),
        },
    );
    assert!(s.indexes.queries.in_flight.is_none());
    assert!(
        reading(&s, parked_tab),
        "the parked tab's read is not the one that failed"
    );
}

#[test]
fn a_parked_tab_claims_the_answer_when_the_current_tab_is_not_waiting() {
    let (s, a, parked_tab) = both_reading();
    let s = ev(s, level(a, "app"));
    // Only the parked tab waits now: the next failure is its.
    let s = ev(
        s,
        Event::Failed {
            session: a,
            context: "profiling_state",
            error: CoreError::Database("no".into()),
        },
    );
    assert!(!reading(&s, parked_tab));
}

#[test]
fn an_answer_for_another_database_is_not_claimed_by_a_tab_reading_this_one() {
    let (s, a, parked_tab) = both_reading();
    let other = level(a, "elsewhere");
    assert_eq!(owner_of(&s, a, &other), None);
    let s = ev(s, other);
    assert!(reading(&s, parked_tab));
    assert!(
        s.indexes.queries.in_flight.is_some(),
        "and neither is the current tab's"
    );
}

// One definition of what a tab awaits.

fn every_read() -> [Read; 11] {
    use Read::*;
    // Exhaustive: a new read must be listed here, and then in `event_for` and `set_number`.
    let all = [
        Find,
        Explain,
        Estimate,
        IndexList,
        IndexHistory,
        QueryShapes,
        Guard,
        Preview,
        AggregateExplain,
        AggregateCount,
        Run,
    ];
    for read in all {
        match read {
            Find | Explain | Estimate | IndexList | IndexHistory | QueryShapes | Guard
            | Preview | AggregateExplain | AggregateCount | Run => {}
        }
    }
    all
}

/// The answer event of `read` carrying `n`.
fn event_for(read: Read, n: u64) -> Event {
    let (session, ns) = (Uuid::nil(), users());
    match read {
        Read::Find => Event::Results {
            session,
            ns,
            query: Default::default(),
            page: curlybrace_core::store::Page {
                docs: vec![],
                skip: 0,
                has_more: false,
            },
            request: n,
        },
        Read::Explain => Event::Explain {
            session,
            ns,
            query: Default::default(),
            outcome: Err(CoreError::Unreachable),
            request: n,
        },
        Read::Estimate => Event::EstimatedCount {
            session,
            ns,
            count: 0,
            request: n,
        },
        Read::IndexList => Event::Indexes {
            session,
            ns,
            indexes: vec![],
            request: n,
        },
        Read::IndexHistory => Event::IndexHistory {
            session,
            ns,
            range: curlybrace_core::history::ChartRange::Day,
            series: vec![],
            request: n,
        },
        Read::QueryShapes => Event::QueryShapes {
            session,
            ns,
            range: curlybrace_core::history::ChartRange::Day,
            shapes: vec![],
            request: n,
        },
        Read::Guard => Event::GuardChecked {
            session,
            request: n,
            outcome: Err(CoreError::Unreachable),
        },
        Read::Preview => Event::AggregatePreviewed {
            session,
            request: n,
            outcome: Ok(vec![]),
        },
        Read::AggregateExplain => Event::AggregateExplained {
            session,
            request: n,
            ns,
            verbosity: curlybrace_core::explain::ExplainVerbosity::QueryPlanner,
            outcome: Err(CoreError::Unreachable),
        },
        Read::AggregateCount => Event::AggregateCounted {
            session,
            request: n,
            ns,
            stage_index: 0,
            outcome: Ok(0),
        },
        Read::Run => Event::AggregateRan {
            session,
            run: n,
            ns: ns.clone(),
            target: ns,
            outcome: Ok(None),
        },
    }
}

#[test]
fn every_numbered_read_has_an_answer_event_and_a_failure_context() {
    for read in every_read() {
        let event = event_for(read, 5);
        assert_eq!(read_of(&event), Some((read, 5)), "{read:?}");
    }
    for (context, read) in [
        ("find", Read::Find),
        ("explain_find", Read::Explain),
        ("estimated_count", Read::Estimate),
        ("list_indexes", Read::IndexList),
        ("index_history", Read::IndexHistory),
        ("query_shapes", Read::QueryShapes),
        ("guard_check", Read::Guard),
        ("aggregate_preview", Read::Preview),
        ("aggregate_explain", Read::AggregateExplain),
        ("aggregate_count", Read::AggregateCount),
    ] {
        assert_eq!(read_of_failure(context), Some(read), "{context}");
    }
    assert_eq!(
        read_of_failure("aggregate_run"),
        None,
        "a run fails as a write, not a read"
    );
}

#[test]
fn a_tab_awaits_exactly_the_number_it_holds_for_each_read() {
    use crate::app::tabs::TabView;
    let mut checked = 0;
    for read in every_read() {
        let mut view = TabView::default();
        let n = 4242;
        let before = event_for(read, n);
        assert!(!view.awaits(&before), "{read:?}: nothing awaited yet");
        let settable = match read {
            Read::Find => {
                view.issued_request = n;
                true
            }
            Read::Explain => {
                view.verdict_request = n;
                view.verdict = crate::app::VerdictState::Pending {
                    query: Default::default(),
                };
                true
            }
            Read::Estimate => {
                view.indexes.estimate_request = n;
                true
            }
            Read::IndexList | Read::IndexHistory => {
                view.indexes.request = n;
                true
            }
            Read::QueryShapes => {
                view.indexes.queries.request = n;
                true
            }
            Read::Guard => {
                *view.guard = crate::app::GuardState::Checking {
                    request: n,
                    query: Default::default(),
                };
                true
            }
            Read::Preview => {
                view.aggregate.in_flight = Some(n);
                true
            }
            Read::Run => {
                view.aggregate.run.run = n;
                true
            }
            // Their numbers come out of a sequence of the draft's own (see tab_routing tests).
            Read::AggregateExplain | Read::AggregateCount => false,
        };
        if settable {
            checked += 1;
            assert!(view.awaits(&event_for(read, n)), "{read:?}");
            assert!(
                Numbers::of(
                    view.issued_request,
                    view.verdict_request,
                    &view.guard,
                    &view.indexes,
                    &view.aggregate
                )
                .awaits(read, n)
            );
            assert!(
                !view.awaits(&event_for(read, n + 1)),
                "{read:?}: another number"
            );
        }
    }
    assert_eq!(checked, 9);
}

// Closing and switching follow the strip.

fn three_tabs() -> (AppState, Uuid, [TabId; 3]) {
    let t = two_tabs();
    let a = t.a;
    let first = t.s.current_tab_of(a).expect("a tab");
    let (s, third) = t.s.tab_opened(a);
    (s, a, [first, t.second, third.expect("a third tab")])
}

#[test]
fn switching_to_a_tab_makes_it_the_active_and_most_recent_one() {
    let (s, _, [_, _, third]) = three_tabs();
    let s = s.tab_current(third);
    assert_eq!(s.tabs.active, Some(third));
    assert_eq!(s.tabs.mru.first(), Some(&third));
}

#[test]
fn closing_the_current_tab_follows_the_strips_neighbour_rules() {
    let (s, a, [first, second, third]) = three_tabs();
    // The last tab closes: the left neighbour takes over.
    let s = s.tab_current(third).tab_closed(third);
    assert_eq!(s.current_tab_of(a), Some(second));
    // The first tab closes: the right neighbour takes over.
    let s = s.tab_current(first).tab_closed(first);
    assert_eq!(s.current_tab_of(a), Some(second));
    assert_eq!(s.tabs_of(a), vec![second]);
}

#[test]
fn closing_a_tab_whose_sibling_has_no_view_does_not_loop_or_touch_the_foreground() {
    let t = two_tabs();
    let a = t.a;
    let first = t.s.current_tab_of(a).expect("a tab");
    let mut s = t.s.tab_closed(t.second);
    // A sibling the strip lists but nobody parked (a state that must not happen, and must not hang).
    s.tabs = std::mem::take(&mut s.tabs).opened_for(a, None).0;
    let page = s.page.as_ref().map(|p| p.docs.len());
    let in_front = s.selected_ns().cloned();
    let s = s.tab_closed(first);
    assert_eq!(
        s.selected_ns().cloned(),
        in_front,
        "the foreground is not corrupted"
    );
    assert_eq!(s.page.as_ref().map(|p| p.docs.len()), page);
    assert_eq!(
        s.current_tab_of(a),
        Some(first),
        "nothing could take over: it stays"
    );
}
