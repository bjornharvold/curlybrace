//! The Indexes tab in the reducer: requests, stale answers, per-session state and failures.
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{CollectionTab, IndexesAction, IndexesState, SidebarAction, TtlPrompt};
use curlybrace_core::error::CoreError;
use curlybrace_core::history::{Bucket, ChartRange, Sample};
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::runtime::{
    Command, EMBEDDED_SESSION, Event, IndexSeries, SamplerPause, SamplerStatus,
};
use curlybrace_core::store::{CollectionIndex, Namespace};
use curlybrace_core::test_support::mock_objects::{mock_index_spec, mock_index_spec_named};
use uuid::Uuid;

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

fn other_ns() -> Namespace {
    Namespace::new("app", "pets")
}

/// Connected embedded session with `app.people` selected.
fn connected() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into(), "pets".into()])
        .toggled("app")
        .selecting(ns());
    s
}

fn listed(name: &str) -> CollectionIndex {
    CollectionIndex {
        spec: mock_index_spec_named(name, &[("a", KeyKind::Asc)]),
        size_bytes: 4096,
    }
}

fn series_of(name: &str) -> IndexSeries {
    IndexSeries {
        index_name: name.into(),
        definition_hash: "h".into(),
        host: "node1:27017".into(),
        buckets: vec![Bucket {
            start_secs: 0,
            width_secs: 600,
            uses: 3,
            restarts: 0,
            gap_secs: 0,
        }],
        latest: Some(Sample {
            at_secs: 10,
            ops: 3,
            since_ms: 1,
        }),
        last_used_at_secs: Some(10),
        tracked_since_secs: Some(0),
    }
}

/// The two read commands for `ns` over `range`, in the order they are sent, numbered `request`.
fn reads(session: Uuid, ns: Namespace, range: ChartRange, request: u64) -> Vec<Command> {
    vec![
        Command::ListIndexes {
            session,
            ns: ns.clone(),
            request,
        },
        Command::IndexHistory {
            session,
            ns,
            range,
            request,
        },
    ]
}

/// `Command` is not `PartialEq`; its `Debug` text is a faithful stand-in.
fn shown(commands: &[Command]) -> String {
    format!("{commands:?}")
}

fn open_tab(s: AppState) -> (AppState, Vec<Command>) {
    update(s, Input::Indexes(IndexesAction::OpenTab))
}

fn event(s: AppState, ev: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(ev))
}

#[test]
fn opening_the_tab_sends_list_indexes_and_index_history_for_the_selected_collection() {
    let (s, commands) = open_tab(connected());
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Day, 1))
    );
    assert_eq!(s.tab, CollectionTab::Indexes);
    assert_eq!(s.indexes.ns, Some(ns()));
    assert!(s.indexes.loading);

    let (s, commands) = update(s, Input::Indexes(IndexesAction::SetRange(ChartRange::Week)));
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Week, 2))
    );
    assert_eq!(s.indexes.range, ChartRange::Week);

    let (_, commands) = update(s, Input::Indexes(IndexesAction::Refresh));
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Week, 3))
    );
}

#[test]
fn switching_to_the_indexes_tab_behaves_like_opening_it() {
    let (s, commands) = update(connected(), Input::Tab(CollectionTab::Indexes));
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Day, 1))
    );
    assert_eq!(s.tab, CollectionTab::Indexes);

    let (s, commands) = update(s, Input::Tab(CollectionTab::Documents));
    assert!(commands.is_empty());
    assert_eq!(s.tab, CollectionTab::Documents);
}

#[test]
fn a_matching_answer_is_applied() {
    let (s, _) = open_tab(connected());
    let (s, _) = event(
        s,
        Event::Indexes {
            session: EMBEDDED_SESSION,
            ns: ns(),
            indexes: vec![listed("a_1")],
            request: 1,
        },
    );
    let (s, _) = event(
        s,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: ns(),
            range: ChartRange::Day,
            series: vec![series_of("a_1")],
            request: 1,
        },
    );
    assert_eq!(s.indexes.indexes, vec![listed("a_1")]);
    assert_eq!(s.indexes.series, vec![series_of("a_1")]);
    assert!(!s.indexes.loading);
}

#[test]
fn a_result_for_another_collection_or_range_is_dropped() {
    let (s, _) = open_tab(connected());
    let (s, _) = event(
        s,
        Event::Indexes {
            session: EMBEDDED_SESSION,
            ns: other_ns(),
            indexes: vec![listed("stale")],
            request: 1,
        },
    );
    let (s, _) = event(
        s,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: other_ns(),
            range: ChartRange::Day,
            series: vec![series_of("stale")],
            request: 1,
        },
    );
    let (s, _) = event(
        s,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: ns(),
            range: ChartRange::Month,
            series: vec![series_of("stale")],
            request: 1,
        },
    );
    assert!(s.indexes.indexes.is_empty());
    assert!(s.indexes.series.is_empty());
    assert!(s.indexes.loading, "the awaited answer has not come yet");
}

#[test]
fn switching_collection_resets_the_indexes_state() {
    let (s, _) = open_tab(connected());
    let (s, _) = event(
        s,
        Event::Indexes {
            session: EMBEDDED_SESSION,
            ns: ns(),
            indexes: vec![listed("a_1")],
            request: 1,
        },
    );
    let s = AppState {
        indexes: s
            .indexes
            .selecting("a_1".into())
            .with_range(ChartRange::Week)
            .with_sampler(SamplerStatus::Running),
        ..s
    };
    let (s, commands) = update(s, Input::Sidebar(SidebarAction::Open(other_ns())));
    assert_eq!(s.indexes.ns, Some(other_ns()));
    assert!(s.indexes.indexes.is_empty());
    assert_eq!(s.indexes.selected, None);
    assert_eq!(
        s.indexes.range,
        ChartRange::Week,
        "the range is a preference"
    );
    assert_eq!(
        s.indexes.sampler,
        Some(SamplerStatus::Running),
        "the sampler belongs to the session"
    );
    assert!(
        s.indexes.loading,
        "the Indexes tab is current: re-requested"
    );
    assert_eq!(
        shown(&commands[1..]),
        shown(&reads(EMBEDDED_SESSION, other_ns(), ChartRange::Week, 2))
    );
    assert!(matches!(commands[0], Command::Find { .. }));
}

#[test]
fn switching_collection_on_the_documents_tab_requests_no_lists() {
    let (s, commands) = update(connected(), Input::Sidebar(SidebarAction::Open(other_ns())));
    assert_eq!(
        s.indexes,
        IndexesState::default().for_collection(other_ns())
    );
    assert!(commands.iter().all(|c| matches!(c, Command::Find { .. })));
}

#[test]
fn each_session_keeps_its_own_tab_and_indexes_state() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let view_b = SessionView {
        status: ConnStatus::Connected,
        tab: CollectionTab::Indexes,
        indexes: IndexesState::default()
            .for_collection(other_ns())
            .selecting("b_1".into()),
        ..SessionView::default()
    };
    let mut s = AppState::new(vec![]);
    s.status = ConnStatus::Connected;
    s.tab = CollectionTab::Documents;
    s.indexes = IndexesState::default().for_collection(ns());
    let s = AppState {
        open: vec![a],
        active: Some(a),
        ..s
    };
    let s = s.opened_with(b, view_b);
    assert_eq!(s.active, Some(b));
    assert_eq!(s.tab, CollectionTab::Indexes);
    assert_eq!(s.indexes.ns, Some(other_ns()));
    assert_eq!(s.parked[&a].tab, CollectionTab::Documents);
    assert_eq!(s.parked[&a].indexes.ns, Some(ns()));

    let s = s.activated(a);
    assert_eq!(s.tab, CollectionTab::Documents);
    assert_eq!(s.indexes.ns, Some(ns()));
    assert_eq!(s.parked[&b].tab, CollectionTab::Indexes);
    assert_eq!(s.parked[&b].indexes.selected.as_deref(), Some("b_1"));
}

#[test]
fn the_sampler_event_updates_the_status_for_its_session_only() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let connected_view = || SessionView {
        status: ConnStatus::Connected,
        ..SessionView::default()
    };
    let s = AppState::new(vec![])
        .opened_with(a, connected_view())
        .opened_with(b, connected_view());
    let s = s.activated(a);
    let paused = SamplerStatus::Paused(SamplerPause::NotAuthorized);
    let (s, commands) = event(
        s,
        Event::Sampler {
            session: b,
            status: paused,
        },
    );
    assert!(commands.is_empty());
    assert_eq!(s.active, Some(a), "a background answer keeps the focus");
    assert_eq!(s.indexes.sampler, None);
    assert_eq!(s.parked[&b].indexes.sampler, Some(paused));

    let (s, _) = event(
        s,
        Event::Sampler {
            session: a,
            status: SamplerStatus::Running,
        },
    );
    assert_eq!(s.indexes.sampler, Some(SamplerStatus::Running));
    assert_eq!(s.parked[&b].indexes.sampler, Some(paused));
}

#[test]
fn a_sampler_event_for_a_session_that_is_not_connected_is_ignored() {
    // Closed session: the late event arrives after `Disconnected`.
    let closed = Uuid::from_u128(9);
    let (s, _) = event(
        connected(),
        Event::Sampler {
            session: closed,
            status: SamplerStatus::Running,
        },
    );
    assert_eq!(s.indexes.sampler, None);
    assert!(!s.parked.contains_key(&closed));

    // Open but no longer connected (its connection failed).
    let failed = AppState {
        status: ConnStatus::Failed,
        ..connected()
    };
    let (s, _) = event(
        failed,
        Event::Sampler {
            session: EMBEDDED_SESSION,
            status: SamplerStatus::Running,
        },
    );
    assert_eq!(s.indexes.sampler, None);
}

#[test]
fn an_index_change_refreshes_the_current_collection_only() {
    let (s, _) = open_tab(connected());
    let (s, commands) = event(
        s,
        Event::IndexChanged {
            session: EMBEDDED_SESSION,
            ns: ns(),
        },
    );
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Day, 2))
    );

    let (_, commands) = event(
        s,
        Event::IndexChanged {
            session: EMBEDDED_SESSION,
            ns: other_ns(),
        },
    );
    assert!(commands.is_empty());
}

#[test]
fn a_failure_in_list_indexes_sets_the_failure_banner() {
    let (s, _) = open_tab(connected());
    assert!(s.indexes.loading);
    let (s, _) = event(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "list_indexes",
            error: CoreError::NotFound,
        },
    );
    assert_eq!(
        s.banner,
        Some(Banner::Failure {
            context: "list_indexes",
            error: CoreError::NotFound,
        })
    );
    assert!(!s.indexes.loading);
}

#[test]
fn the_tab_is_gated_on_a_connected_session() {
    let idle = AppState {
        status: ConnStatus::Disconnected,
        ..connected()
    };
    for input in [
        Input::Indexes(IndexesAction::OpenTab),
        Input::Indexes(IndexesAction::Refresh),
        Input::Indexes(IndexesAction::SetRange(ChartRange::Month)),
        Input::Tab(CollectionTab::Indexes),
    ] {
        let (s, commands) = update(
            AppState {
                status: idle.status,
                ..connected()
            },
            input,
        );
        assert!(commands.is_empty());
        assert_eq!(s.banner, Some(Banner::NotConnected));
        assert_eq!(s.tab, CollectionTab::Documents);
    }
}

#[test]
fn selecting_and_copying_update_state_only() {
    let (s, _) = open_tab(connected());
    let (s, commands) = update(s, Input::Indexes(IndexesAction::Select("a_1".into())));
    assert!(commands.is_empty());
    assert_eq!(s.indexes.selected.as_deref(), Some("a_1"));
    let before = s.indexes.clone();
    let (s, commands) = update(
        s,
        Input::Indexes(IndexesAction::CopyJson(serde_json_text(
            &mock_index_spec().name,
        ))),
    );
    assert!(commands.is_empty());
    assert_eq!(s.indexes, before);
}

fn serde_json_text(name: &str) -> String {
    format!("{{\"name\":\"{name}\"}}")
}

fn listing(request: u64, names: &[&str]) -> Event {
    Event::Indexes {
        session: EMBEDDED_SESSION,
        ns: ns(),
        indexes: names.iter().map(|n| listed(n)).collect(),
        request,
    }
}

fn history_of(request: u64, names: &[&str]) -> Event {
    Event::IndexHistory {
        session: EMBEDDED_SESSION,
        ns: ns(),
        range: ChartRange::Day,
        series: names.iter().map(|n| series_of(n)).collect(),
        request,
    }
}

#[test]
fn an_older_listing_arriving_last_does_not_replace_the_newer_one() {
    let (s, _) = open_tab(connected());
    let (s, commands) = update(s, Input::Indexes(IndexesAction::Refresh));
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Day, 2))
    );

    // The second request answers first; the first one's answer follows and is stale.
    let (s, _) = event(s, listing(2, &["a_1", "b_1"]));
    let (s, _) = event(s, history_of(2, &["a_1", "b_1"]));
    let (s, _) = event(s, listing(1, &["a_1"]));
    let (s, _) = event(s, history_of(1, &["a_1"]));

    assert_eq!(s.indexes.indexes, vec![listed("a_1"), listed("b_1")]);
    assert_eq!(s.indexes.series, vec![series_of("a_1"), series_of("b_1")]);
    assert!(!s.indexes.loading);
}

#[test]
fn an_older_answer_does_not_end_the_wait_for_the_newer_one() {
    let (s, _) = open_tab(connected());
    let (s, _) = update(s, Input::Indexes(IndexesAction::Refresh));

    let (s, _) = event(s, listing(1, &["a_1"]));
    let (s, _) = event(s, history_of(1, &["a_1"]));

    assert!(s.indexes.indexes.is_empty() && s.indexes.series.is_empty());
    assert!(s.indexes.loading, "request 2 is still awaited");
    let (s, _) = event(s, listing(2, &["a_1", "b_1"]));
    assert_eq!(s.indexes.indexes.len(), 2);
    assert!(!s.indexes.loading);
}

#[test]
fn a_request_number_is_never_reused_across_collections() {
    let (s, _) = open_tab(connected());
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(other_ns())));
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(ns())));
    assert_eq!(
        s.indexes.request, 3,
        "one per request, kept across collections"
    );
    // The answer of the first visit to `ns` is not the awaited one of the second.
    let (s, _) = event(s, listing(1, &["stale"]));
    assert!(s.indexes.indexes.is_empty());
}

fn read_failed(request: u64, context: &'static str) -> Event {
    Event::RequestFailed {
        session: EMBEDDED_SESSION,
        context,
        request,
        error: CoreError::NotFound,
    }
}

/// Request 1 answered, request 2 (a refresh) still in flight.
fn awaiting_the_second_request() -> AppState {
    let (s, _) = open_tab(connected());
    let (s, _) = update(s, Input::Indexes(IndexesAction::Refresh));
    event(s, listing(1, &["old"])).0
}

#[test]
fn a_late_failure_of_an_older_request_neither_ends_the_wait_nor_raises_a_banner() {
    for context in ["list_indexes", "index_history"] {
        let s = awaiting_the_second_request();
        let before = s.indexes.clone();
        let (after, commands) = event(s, read_failed(1, context));
        assert!(commands.is_empty());
        assert!(after.indexes.loading, "request 2 is still awaited");
        assert_eq!(
            after.banner, None,
            "{context}: no banner over the newer data"
        );
        assert_eq!(after.indexes, before);
    }
}

#[test]
fn a_failure_of_the_newest_request_settles_it_and_raises_the_banner() {
    let (s, _) = open_tab(connected());
    let (s, _) = update(s, Input::Indexes(IndexesAction::Refresh));

    let (s, _) = event(s, read_failed(2, "list_indexes"));

    assert!(!s.indexes.loading);
    assert_eq!(
        s.banner,
        Some(Banner::Failure {
            context: "list_indexes",
            error: CoreError::NotFound,
        })
    );
}

#[test]
fn a_late_lost_connection_failure_of_an_older_request_still_reaches_connection_lost() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let s = awaiting_the_second_request();
        let late = Event::RequestFailed {
            session: EMBEDDED_SESSION,
            context: "list_indexes",
            request: 1,
            error: error.clone(),
        };

        let (after, _) = event(s, late);

        assert_eq!(after.failure, Some(error.clone()), "{error:?}");
    }
}

fn late_lost(request: u64, error: CoreError) -> Event {
    Event::RequestFailed {
        session: EMBEDDED_SESSION,
        context: "list_indexes",
        request,
        error,
    }
}

#[test]
fn a_late_lost_connection_of_an_older_request_is_ignored_once_a_newer_one_was_answered() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let (s, _) = open_tab(connected());
        let (s, _) = update(s, Input::Indexes(IndexesAction::Refresh));
        let (s, _) = event(s, listing(2, &["a_1"]));
        let before = s.indexes.clone();

        let (after, commands) = event(s, late_lost(1, error.clone()));

        assert!(commands.is_empty());
        assert_eq!(after.failure, None, "{error:?}: the session just answered");
        assert_eq!(after.banner, None);
        assert_eq!(after.indexes, before);
    }
}

#[test]
fn a_late_lost_connection_with_a_prompt_open_does_not_end_the_wait_for_the_newest_request() {
    let (s, _) = open_tab(connected());
    let (s, _) = event(s, listing(1, &["a_1"]));
    let (s, _) = update(s, Input::Indexes(IndexesAction::Refresh));
    let mut s = s;
    s.indexes.ttl_prompt = Some(TtlPrompt::new("a_1".into(), Some(100)));

    let (after, _) = event(s, late_lost(1, CoreError::Unreachable));

    assert!(after.indexes.loading, "request 2 is still awaited");
}
