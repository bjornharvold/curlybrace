//! The Indexes tab in the reducer: requests, stale answers, per-session state and failures.
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{CollectionTab, IndexesAction, IndexesState, SidebarAction};
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

/// The two read commands for `ns` over `range`, in the order they are sent.
fn reads(session: Uuid, ns: Namespace, range: ChartRange) -> Vec<Command> {
    vec![
        Command::ListIndexes {
            session,
            ns: ns.clone(),
        },
        Command::IndexHistory { session, ns, range },
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
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Day))
    );
    assert_eq!(s.tab, CollectionTab::Indexes);
    assert_eq!(s.indexes.ns, Some(ns()));
    assert!(s.indexes.loading);

    let (s, commands) = update(s, Input::Indexes(IndexesAction::SetRange(ChartRange::Week)));
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Week))
    );
    assert_eq!(s.indexes.range, ChartRange::Week);

    let (_, commands) = update(s, Input::Indexes(IndexesAction::Refresh));
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Week))
    );
}

#[test]
fn switching_to_the_indexes_tab_behaves_like_opening_it() {
    let (s, commands) = update(connected(), Input::Tab(CollectionTab::Indexes));
    assert_eq!(
        shown(&commands),
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Day))
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
        },
    );
    let (s, _) = event(
        s,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: ns(),
            range: ChartRange::Day,
            series: vec![series_of("a_1")],
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
        },
    );
    let (s, _) = event(
        s,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: other_ns(),
            range: ChartRange::Day,
            series: vec![series_of("stale")],
        },
    );
    let (s, _) = event(
        s,
        Event::IndexHistory {
            session: EMBEDDED_SESSION,
            ns: ns(),
            range: ChartRange::Month,
            series: vec![series_of("stale")],
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
        shown(&reads(EMBEDDED_SESSION, other_ns(), ChartRange::Week))
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
        shown(&reads(EMBEDDED_SESSION, ns(), ChartRange::Day))
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
