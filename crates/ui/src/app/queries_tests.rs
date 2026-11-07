//! The Queries view in the reducer: opening it, the consent flow of the profiler and what the
//! runtime answers.
use super::index_write_tests::{browsing, ns, plain, shown, with_flags};
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, Banner, ConnStatus};
use super::write_guard::{WriteKind, WriteWarningAction};
use crate::viewmodel::{
    IndexesAction, IndexesState, IndexesView, ProfilerConsent, ProfilingView, QueriesAction,
    SidebarState,
};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ChartRange, ShapeSource, ShapeSummary};
use curlybrace_core::runtime::{Command, Event, ProfilerStatus, SamplerPause};
use curlybrace_core::store::{Namespace, PROFILER_MONGOS_MESSAGE, ProfilingState};
use uuid::Uuid;

const PROFILER_ON: i32 = 2;

fn queries(s: AppState, action: QueriesAction) -> (AppState, Vec<Command>) {
    update(s, Input::Indexes(IndexesAction::Queries(action)))
}

fn event(s: AppState, ev: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(ev))
}

fn failure() -> CoreError {
    CoreError::Database("timed out".into())
}

fn level(level: i32) -> ProfilingState {
    ProfilingState {
        level,
        slow_ms: 100,
    }
}

fn answer(session: Uuid, db: &str, outcome: Result<ProfilingState, CoreError>) -> Event {
    Event::ProfilingState {
        session,
        db: db.into(),
        outcome,
    }
}

fn shape(hash: &str) -> ShapeSummary {
    ShapeSummary {
        hash: hash.into(),
        shape_json: "{}".into(),
        source: ShapeSource::App,
        executions: 3,
        misses: 1,
        total_millis: 30,
        max_millis: 20,
        worst: VerdictKind::CollectionScan,
        buckets: Vec::new(),
    }
}

fn shapes_event(session: Uuid, range: ChartRange, request: u64, hashes: &[&str]) -> Event {
    Event::QueryShapes {
        session,
        ns: ns(),
        range,
        shapes: hashes.iter().map(|h| shape(h)).collect(),
        request,
    }
}

fn flags(production: bool, read_only: bool) -> ConnectionConfig {
    with_flags(production, read_only)
}

/// The Queries view opened on `app.people` of a connection with these flags.
fn opened(cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = browsing(cfg);
    let (s, _) = queries(s, QueriesAction::Open);
    (s, id)
}

/// Opened, and the profiler state answered: `current` is the level the server reports.
fn ready(cfg: ConnectionConfig, current: i32) -> (AppState, Uuid) {
    let (s, id) = opened(cfg);
    let (s, _) = event(s, answer(id, "app", Ok(level(current))));
    (s, id)
}

fn asked(cfg: ConnectionConfig, enable: bool) -> (AppState, Uuid) {
    let (s, id) = ready(cfg, if enable { 0 } else { PROFILER_ON });
    let action = match enable {
        true => QueriesAction::AskEnable,
        false => QueriesAction::AskDisable,
    };
    (queries(s, action).0, id)
}

fn set_level(session: Uuid, level: i32) -> Command {
    Command::SetProfilingLevel {
        session,
        db: "app".into(),
        level,
    }
}

#[test]
fn opening_the_queries_view_asks_for_profiling_state_and_shapes() {
    let (s, id) = plain();
    let (s, commands) = queries(s, QueriesAction::Open);
    assert_eq!(s.indexes.view, IndexesView::Queries);
    assert_eq!(s.indexes.queries.request, 1);
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Loading);
    assert_eq!(
        shown(&commands),
        shown(&[
            Command::QueryShapes {
                session: id,
                ns: ns(),
                range: ChartRange::Day,
                request: 1
            },
            Command::ProfilingState {
                session: id,
                db: "app".into()
            },
        ])
    );
    let (s, _) = event(s, answer(id, "app", Ok(level(0))));
    let (s, commands) = update(
        s,
        Input::Indexes(IndexesAction::ShowView(IndexesView::Indexes)),
    );
    assert_eq!(s.indexes.view, IndexesView::Indexes);
    assert!(commands.is_empty());
    let (s, commands) = update(
        s,
        Input::Indexes(IndexesAction::ShowView(IndexesView::Queries)),
    );
    assert_eq!(s.indexes.view, IndexesView::Queries);
    assert_eq!(commands.len(), 2);
}

#[test]
fn opening_needs_a_connection_but_not_a_writable_one() {
    let (s, _) = browsing(flags(false, true));
    let (s, commands) = queries(s, QueriesAction::Open);
    assert_eq!(
        commands.len(),
        2,
        "a read is allowed on a read-only connection"
    );
    assert!(s.banner.is_none());
    let s = AppState {
        status: ConnStatus::Connecting,
        ..s
    };
    let (s, commands) = queries(s, QueriesAction::Open);
    assert!(commands.is_empty());
    assert!(matches!(s.banner, Some(Banner::NotConnected)));
}

#[test]
fn refresh_and_range_change_bump_the_request() {
    let (s, id) = ready(flags(false, false), 0);
    let (s, commands) = queries(s, QueriesAction::Refresh);
    assert_eq!(s.indexes.queries.request, 2);
    assert_eq!(commands.len(), 2);
    let (s, _) = event(s, answer(id, "app", Ok(level(0))));
    let (s, commands) = queries(s, QueriesAction::SetRange(ChartRange::Week));
    assert_eq!(s.indexes.queries.request, 3);
    assert_eq!(s.indexes.queries.range, ChartRange::Week);
    assert_eq!(
        shown(&commands[..1]),
        shown(&[Command::QueryShapes {
            session: id,
            ns: ns(),
            range: ChartRange::Week,
            request: 3
        }])
    );
}

#[test]
fn enable_asks_consent_and_sends_nothing() {
    let (s, _) = ready(flags(false, false), 0);
    let (s, commands) = queries(s, QueriesAction::AskEnable);
    assert!(commands.is_empty());
    assert_eq!(
        s.indexes.queries.consent,
        Some(ProfilerConsent {
            db: "app".into(),
            level: 2
        })
    );
    assert!(s.indexes.has_prompt());
    assert!(!s.indexes.queries.writing());
}

#[test]
fn disable_asks_consent_and_sends_nothing() {
    let (s, _) = ready(flags(false, false), PROFILER_ON);
    let (s, commands) = queries(s, QueriesAction::AskDisable);
    assert!(commands.is_empty());
    assert_eq!(
        s.indexes.queries.consent,
        Some(ProfilerConsent {
            db: "app".into(),
            level: 0
        })
    );
    assert!(!s.indexes.queries.writing());
}

#[test]
fn asking_and_cancelling_never_send_a_command() {
    let (mut s, _) = ready(flags(false, false), 0);
    for action in [
        QueriesAction::AskEnable,
        QueriesAction::CancelEnable,
        QueriesAction::AskDisable,
        QueriesAction::CancelEnable,
    ] {
        let (next, commands) = queries(s, action);
        assert!(commands.is_empty());
        assert!(!next.indexes.queries.writing());
        s = next;
    }
}

#[test]
fn confirm_sends_set_profiling_level_2_for_the_namespace_db() {
    let (s, id) = asked(flags(false, false), true);
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert_eq!(shown(&commands), shown(&[set_level(id, 2)]));
    assert!(s.indexes.queries.writing());
    assert!(s.pending.is_some());
    // The prompt stays until the answer, like the drop confirmation.
    assert!(s.indexes.queries.consent.is_some());
    // A second confirm while the write is in flight sends nothing.
    let (_, again) = queries(s, QueriesAction::ConfirmEnable);
    assert!(again.is_empty());
}

#[test]
fn confirm_disable_sends_level_0() {
    let (s, id) = asked(flags(false, false), false);
    let (s, commands) = queries(s, QueriesAction::ConfirmDisable);
    assert_eq!(shown(&commands), shown(&[set_level(id, 0)]));
    assert!(s.indexes.queries.writing());
}

#[test]
fn a_confirm_that_does_not_match_the_prompt_sends_nothing() {
    let (s, _) = asked(flags(false, false), true);
    let (_, commands) = queries(s, QueriesAction::ConfirmDisable);
    assert!(commands.is_empty());
    let (s, _) = opened(flags(false, false));
    let (_, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert!(commands.is_empty(), "no prompt, nothing to confirm");
}

#[test]
fn cancel_sends_nothing_and_closes_the_prompt() {
    let (s, _) = asked(flags(false, false), true);
    let (s, commands) = queries(s, QueriesAction::CancelEnable);
    assert!(commands.is_empty());
    assert_eq!(s.indexes.queries.consent, None);
    assert!(!s.indexes.has_prompt());
}

#[test]
fn read_only_connection_refuses_confirm_with_the_read_only_banner_and_no_command() {
    // Even with a prompt somehow open, the guard refuses.
    let (s, _) = ready(flags(false, true), 0);
    let consent = Some(ProfilerConsent {
        db: "app".into(),
        level: 2,
    });
    let held = s.indexes.queries.clone().with_consent(consent);
    let s = AppState {
        indexes: s.indexes.clone().with_queries(held),
        ..s
    };
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert!(commands.is_empty());
    assert!(matches!(s.banner, Some(Banner::ReadOnly)));
    assert!(!s.indexes.queries.writing());
    assert!(s.pending.is_none());
}

#[test]
fn asking_on_a_read_only_connection_is_refused_without_a_prompt() {
    for ask in [QueriesAction::AskEnable, QueriesAction::AskDisable] {
        let (s, _) = ready(flags(false, true), 0);
        let (s, commands) = queries(s, ask);
        assert!(commands.is_empty());
        assert!(matches!(s.banner, Some(Banner::ReadOnly)));
        assert_eq!(s.indexes.queries.consent, None);
        assert!(!s.indexes.has_prompt());
    }
}

#[test]
fn production_session_parks_the_write_warning_first() {
    let (s, id) = asked(flags(true, false), true);
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert!(commands.is_empty(), "nothing is sent before the warning");
    let parked = s.write_warning.as_ref().expect("the warning is open");
    assert_eq!(parked.kind, WriteKind::SetProfilingLevel);
    assert!(!s.indexes.queries.writing());
    assert!(s.indexes.queries.consent.is_some());

    let (confirmed, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(shown(&commands), shown(&[set_level(id, 2)]));
    assert!(confirmed.indexes.queries.writing());
}

#[test]
fn cancelling_the_production_warning_keeps_the_prompt_and_sends_nothing() {
    let (s, _) = asked(flags(true, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    let (cancelled, commands) = update(s, Input::WriteWarning(WriteWarningAction::Cancel));
    assert!(commands.is_empty());
    assert!(!cancelled.indexes.queries.writing());
    assert!(cancelled.indexes.queries.consent.is_some());
}

#[test]
fn a_successful_set_updates_the_state_and_closes_the_prompt() {
    let (s, id) = asked(flags(false, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    let (s, commands) = event(s, answer(id, "app", Ok(level(2))));
    assert!(commands.is_empty());
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Known(level(2)));
    assert!(!s.indexes.queries.writing());
    assert_eq!(s.indexes.queries.consent, None);
    assert_eq!(s.indexes.queries.error, None);
    assert!(s.pending.is_none(), "the write is settled");
}

#[test]
fn stale_query_shapes_answer_is_dropped() {
    let (s, id) = opened(flags(false, false));
    // Wrong request.
    let (s, _) = event(s, shapes_event(id, ChartRange::Day, 99, &["a"]));
    assert!(s.indexes.queries.shapes.is_empty());
    // Wrong range.
    let (s, _) = event(s, shapes_event(id, ChartRange::Month, 1, &["a"]));
    assert!(s.indexes.queries.shapes.is_empty());
    // Wrong namespace.
    let other = Event::QueryShapes {
        session: id,
        ns: Namespace::new("app", "other"),
        range: ChartRange::Day,
        shapes: vec![shape("a")],
        request: 1,
    };
    let (s, _) = event(s, other);
    assert!(s.indexes.queries.shapes.is_empty());
    assert!(s.indexes.queries.loading);
    // The awaited one.
    let (s, _) = event(s, shapes_event(id, ChartRange::Day, 1, &["a", "b"]));
    assert_eq!(s.indexes.queries.shapes.len(), 2);
    assert!(!s.indexes.queries.loading);
    // After a refresh the older answer is stale.
    let (s, _) = queries(s, QueriesAction::Refresh);
    let (s, _) = event(s, shapes_event(id, ChartRange::Day, 1, &["old"]));
    assert_eq!(s.indexes.queries.shapes.len(), 2);
    assert_eq!(s.indexes.queries.shapes[0].hash, "a");
}

#[test]
fn stale_profiling_state_for_another_db_is_ignored() {
    let (s, id) = opened(flags(false, false));
    let (s, commands) = event(s, answer(id, "elsewhere", Ok(level(2))));
    assert!(commands.is_empty());
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Loading);
}

#[test]
fn profiler_unavailable_outcomes_keep_the_view_usable() {
    for error in [
        CoreError::Unauthorized,
        CoreError::Unsupported(PROFILER_MONGOS_MESSAGE.into()),
    ] {
        let (s, id) = opened(flags(false, false));
        let (s, commands) = event(s, answer(id, "app", Err(error.clone())));
        assert!(commands.is_empty());
        assert_eq!(
            s.indexes.queries.profiling,
            ProfilingView::Unavailable(error)
        );
        assert!(s.banner.is_none(), "never a banner");
        // The shapes still arrive and refresh still works.
        let (s, _) = event(s, shapes_event(id, ChartRange::Day, 1, &["a"]));
        assert_eq!(s.indexes.queries.shapes.len(), 1);
        let (_, commands) = queries(s, QueriesAction::Refresh);
        assert_eq!(commands.len(), 2);
    }
}

#[test]
fn read_failure_is_unavailable_not_a_banner() {
    let (s, id) = opened(flags(false, false));
    let (s, _) = event(s, answer(id, "app", Err(failure())));
    assert_eq!(
        s.indexes.queries.profiling,
        ProfilingView::Unavailable(failure())
    );
    assert!(s.banner.is_none());
}

#[test]
fn a_failed_set_is_ambiguous_and_rereads_the_state() {
    let (s, id) = asked(flags(false, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    let (s, commands) = event(s, answer(id, "app", Err(failure())));
    assert_eq!(
        shown(&commands),
        shown(&[Command::ProfilingState {
            session: id,
            db: "app".into()
        }])
    );
    assert!(s.banner.is_none());
    assert_eq!(s.indexes.queries.error, Some(failure()));
    assert!(!s.indexes.queries.writing());
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Loading);
    assert!(s.pending.is_none(), "the write is no longer in flight");
    assert!(
        s.indexes.queries.consent.is_some(),
        "the person may try again"
    );
    // The re-read settles what the server really holds.
    let (s, _) = event(s, answer(id, "app", Ok(level(2))));
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Known(level(2)));
    assert_eq!(s.indexes.queries.error, None);
    assert_eq!(s.indexes.queries.consent, None);
}

#[test]
fn refresh_during_a_write_does_not_reread_the_profiling_state() {
    let (s, id) = asked(flags(false, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    let (_, commands) = queries(s, QueriesAction::Refresh);
    assert!(
        matches!(commands.as_slice(), [Command::QueryShapes { session, .. }] if *session == id)
    );
}

#[test]
fn a_failed_shapes_read_is_inline_and_settles_only_the_awaited_request() {
    let (s, id) = opened(flags(false, false));
    let failed = |request| Event::RequestFailed {
        session: id,
        context: "query_shapes",
        request,
        error: failure(),
    };
    // An older request's failure changes nothing.
    let (s, _) = queries(s, QueriesAction::Refresh);
    let (s, _) = event(s, failed(1));
    assert!(s.indexes.queries.loading);
    assert_eq!(s.indexes.queries.error, None);
    // The awaited one settles with an inline error and no banner.
    let (s, commands) = event(s, failed(2));
    assert!(commands.is_empty());
    assert!(!s.indexes.queries.loading);
    assert_eq!(s.indexes.queries.error, Some(failure()));
    assert!(s.banner.is_none());
    assert!(s.failure.is_none());
}

#[test]
fn profiler_status_event_is_stored_and_idle_when_absent() {
    let (s, id) = opened(flags(false, false));
    assert_eq!(s.indexes.queries.profiler_status(), ProfilerStatus::Idle);
    let (s, commands) = event(
        s,
        Event::Profiler {
            session: id,
            status: ProfilerStatus::Reading { databases: 2 },
        },
    );
    assert!(commands.is_empty());
    assert_eq!(
        s.indexes.queries.status,
        Some(ProfilerStatus::Reading { databases: 2 })
    );
}

fn view_of() -> SessionView {
    let sidebar = SidebarState::default()
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .selecting(ns());
    SessionView {
        status: ConnStatus::Connected,
        sidebar,
        indexes: IndexesState::default().for_collection(ns()),
        ..SessionView::default()
    }
}

#[test]
fn background_session_events_land_in_the_parked_view() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let s = AppState::new(vec![])
        .opened_with(a, view_of())
        .opened_with(b, view_of())
        .activated(a);
    let (s, _) = event(
        s,
        Event::Profiler {
            session: b,
            status: ProfilerStatus::Paused(SamplerPause::NotAuthorized),
        },
    );
    let (s, _) = event(s, answer(b, "app", Ok(level(2))));
    assert_eq!(s.active, Some(a), "a background answer keeps the focus");
    assert_eq!(s.indexes.queries.status, None);
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Unknown);
    let parked = &s.parked[&b].indexes.queries;
    assert_eq!(
        parked.status,
        Some(ProfilerStatus::Paused(SamplerPause::NotAuthorized))
    );
    assert_eq!(parked.profiling, ProfilingView::Known(level(2)));

    // Switching sessions keeps each one's own state.
    let s = s.activated(b);
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Known(level(2)));
    assert_eq!(
        s.parked[&a].indexes.queries.profiling,
        ProfilingView::Unknown
    );
}

#[test]
fn switching_collection_resets_the_queries_state() {
    let (s, id) = opened(flags(false, false));
    let (s, _) = event(s, shapes_event(id, ChartRange::Day, 1, &["a"]));
    let (s, _) = event(s, answer(id, "app", Ok(level(2))));
    let (s, _) = event(
        s,
        Event::Profiler {
            session: id,
            status: ProfilerStatus::Reading { databases: 1 },
        },
    );
    let (s, _) = queries(s, QueriesAction::AskDisable);
    let sidebar = s
        .sidebar
        .clone()
        .with_collections("app", vec!["people".into(), "orders".into()])
        .selecting(Namespace::new("app", "orders"));
    let s = AppState { sidebar, ..s };
    let (s, _) = super::index_actions::collection_opened(s);
    let q = &s.indexes.queries;
    assert!(q.shapes.is_empty());
    assert_eq!(q.profiling, ProfilingView::Unknown);
    assert_eq!(q.consent, None);
    assert_eq!(s.indexes.view, IndexesView::Indexes);
    // The numbering only grows and the status belongs to the session, not the collection.
    assert!(q.request >= 1);
    assert_eq!(q.status, Some(ProfilerStatus::Reading { databases: 1 }));
}

// ---- single-flight profiler commands ----

fn consenting(s: AppState, db: &str, level: i32) -> AppState {
    let held = s
        .indexes
        .queries
        .clone()
        .with_consent(Some(ProfilerConsent {
            db: db.into(),
            level,
        }));
    AppState {
        indexes: s.indexes.clone().with_queries(held),
        ..s
    }
}

/// Selects `ns` in the sidebar and opens it, as a click would.
fn switched_to(s: AppState, ns: Namespace) -> AppState {
    let sidebar = s
        .sidebar
        .clone()
        .with_collections(&ns.db, vec![ns.collection.clone()])
        .selecting(ns);
    super::index_actions::collection_opened(AppState { sidebar, ..s }).0
}

#[test]
fn a_read_in_flight_blocks_ask_and_confirm() {
    let (s, _) = opened(flags(false, false));
    assert!(s.indexes.queries.in_flight.is_some());
    for ask in [QueriesAction::AskEnable, QueriesAction::AskDisable] {
        let (next, commands) = queries(opened(flags(false, false)).0, ask);
        assert!(commands.is_empty());
        assert_eq!(next.indexes.queries.consent, None);
    }
    let s = consenting(s, "app", 2);
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert!(commands.is_empty());
    assert!(!s.indexes.queries.writing());
    assert!(s.pending.is_none());
}

#[test]
fn confirm_while_loading_is_a_noop() {
    let (s, _) = ready(flags(false, false), 0);
    let held = s
        .indexes
        .queries
        .clone()
        .with_profiling(ProfilingView::Loading)
        .with_consent(Some(ProfilerConsent {
            db: "app".into(),
            level: 2,
        }));
    let s = AppState {
        indexes: s.indexes.clone().with_queries(held),
        ..s
    };
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert!(commands.is_empty());
    assert!(!s.indexes.queries.writing());
}

#[test]
fn refresh_during_a_write_does_not_resend_the_profiling_read_but_refreshes_shapes() {
    let (s, id) = asked(flags(false, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    let before = s.indexes.queries.request;
    let (s, commands) = queries(s, QueriesAction::Refresh);
    assert!(
        matches!(commands.as_slice(), [Command::QueryShapes { session, request, .. }]
            if *session == id && *request == before + 1)
    );
    let (s, commands) = queries(s, QueriesAction::SetRange(ChartRange::Week));
    assert_eq!(commands.len(), 1);
    assert!(s.indexes.queries.writing());
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Known(level(0)));
}

#[test]
fn refresh_while_a_read_is_in_flight_sends_one_profiling_read_in_total() {
    let (s, _) = opened(flags(false, false));
    let (s, commands) = queries(s, QueriesAction::Refresh);
    assert!(matches!(commands.as_slice(), [Command::QueryShapes { .. }]));
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Loading);
}

#[test]
fn a_late_read_answer_cannot_overwrite_a_written_level() {
    let (s, id) = ready(flags(false, false), 0);
    let (s, _) = queries(s, QueriesAction::AskEnable);
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert_eq!(commands.len(), 1);
    // The write is the only command in flight, so its answer is attributed to it.
    let (s, _) = event(s, answer(id, "app", Ok(level(2))));
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Known(level(2)));
    assert!(s.indexes.queries.in_flight.is_none());
    // With nothing in flight an answer is simply stored (it can only be the server's current
    // word); it cannot be mistaken for a write's.
    let (s, _) = event(s, answer(id, "app", Ok(level(0))));
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Known(level(0)));
    assert!(!s.indexes.queries.writing());
}

#[test]
fn a_failed_set_rereads_once_and_a_failed_reread_is_unavailable_without_a_further_command() {
    let (s, id) = asked(flags(false, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    let (s, commands) = event(s, answer(id, "app", Err(failure())));
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        s.indexes.queries.in_flight,
        Some(crate::viewmodel::InFlight::Read(_))
    ));
    let (s, commands) = event(s, answer(id, "app", Err(failure())));
    assert!(commands.is_empty(), "no loop");
    assert_eq!(
        s.indexes.queries.profiling,
        ProfilingView::Unavailable(failure())
    );
    assert!(s.indexes.queries.in_flight.is_none());
    assert!(s.banner.is_none());
}

#[test]
fn a_bare_failed_set_command_settles_the_write_and_rereads_without_a_banner() {
    let (s, id) = asked(flags(false, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    assert!(s.pending.is_some());
    let failed = Event::Failed {
        session: id,
        context: "set_profiling_level",
        error: failure(),
    };
    let (s, commands) = event(s, failed);
    assert_eq!(
        shown(&commands),
        shown(&[Command::ProfilingState {
            session: id,
            db: "app".into()
        }])
    );
    assert!(s.banner.is_none());
    assert!(s.pending.is_none());
    assert!(!s.indexes.queries.writing());
    assert_eq!(s.indexes.queries.error, Some(failure()));
}

#[test]
fn a_bare_failed_read_command_is_unavailable_without_a_banner() {
    let (s, id) = opened(flags(false, false));
    let failed = Event::Failed {
        session: id,
        context: "profiling_state",
        error: failure(),
    };
    let (s, commands) = event(s, failed);
    assert!(commands.is_empty());
    assert!(s.banner.is_none());
    assert_eq!(
        s.indexes.queries.profiling,
        ProfilingView::Unavailable(failure())
    );
    assert!(s.indexes.queries.in_flight.is_none());
}

#[test]
fn a_superseded_profiler_failure_changes_nothing() {
    let (s, id) = opened(flags(false, false));
    let failed = Event::Failed {
        session: id,
        context: "profiling_state",
        error: CoreError::Superseded,
    };
    let (s, commands) = event(s, failed);
    assert!(commands.is_empty());
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Loading);
}

#[test]
fn an_answer_for_the_old_database_is_followed_by_a_read_of_the_new_one() {
    let (s, id) = opened(flags(false, false));
    let s = switched_to(s, Namespace::new("other", "x"));
    let (s, commands) = queries(s, QueriesAction::Open);
    assert!(
        matches!(commands.as_slice(), [Command::QueryShapes { .. }]),
        "one profiler command at a time"
    );
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Loading);
    let (s, commands) = event(s, answer(id, "app", Ok(level(2))));
    assert_eq!(
        shown(&commands),
        shown(&[Command::ProfilingState {
            session: id,
            db: "other".into()
        }])
    );
    let (s, _) = event(s, answer(id, "other", Ok(level(0))));
    assert_eq!(s.indexes.queries.profiling, ProfilingView::Known(level(0)));
}

#[test]
fn ask_then_switch_collection_then_confirm_sends_nothing() {
    let (s, _) = asked(flags(false, false), true);
    let s = switched_to(s, Namespace::new("other", "x"));
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert!(commands.is_empty());
    assert!(!s.indexes.queries.writing());
}

#[test]
fn consent_for_another_db_never_confirms() {
    let (s, _) = ready(flags(false, false), 0);
    let s = consenting(s, "other", 2);
    let (s, commands) = queries(s, QueriesAction::ConfirmEnable);
    assert!(commands.is_empty());
    assert!(s.write_warning.is_none() && s.pending.is_none());
}

#[test]
fn production_warning_parked_then_collection_switch_behaves_like_other_parked_writes() {
    let (s, id) = asked(flags(true, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    assert!(s.write_warning.is_some());
    // The parked write keeps what it was aimed at; the new collection's prompt is gone.
    let s = switched_to(s, Namespace::new("other", "x"));
    assert!(s.write_warning.is_some());
    assert_eq!(s.indexes.queries.consent, None);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(shown(&commands), shown(&[set_level(id, 2)]));
    assert!(s.indexes.queries.writing());
    // Its answer settles it even though the person is on another database now.
    let (s, _) = event(s, answer(id, "app", Ok(level(2))));
    assert!(!s.indexes.queries.writing());
    assert!(s.pending.is_none());
    assert_ne!(s.indexes.queries.profiling, ProfilingView::Known(level(2)));
}

#[test]
fn no_profiler_read_is_sent_while_the_production_warning_is_open() {
    let (s, _) = asked(flags(true, false), true);
    let (s, _) = queries(s, QueriesAction::ConfirmEnable);
    let (_, commands) = queries(s, QueriesAction::Refresh);
    assert!(matches!(commands.as_slice(), [Command::QueryShapes { .. }]));
}

#[test]
fn query_shapes_unreachable_still_marks_the_connection_lost() {
    let (s, id) = opened(flags(false, false));
    let failed = Event::RequestFailed {
        session: id,
        context: "query_shapes",
        request: 1,
        error: CoreError::Unreachable,
    };
    let (s, _) = event(s, failed);
    assert!(s.failure.is_some(), "the session is marked lost");
    assert!(!s.indexes.queries.loading);
}

#[test]
fn an_older_unreachable_query_shapes_failure_is_ignored() {
    let (s, id) = opened(flags(false, false));
    let (s, _) = queries(s, QueriesAction::Refresh);
    let failed = Event::RequestFailed {
        session: id,
        context: "query_shapes",
        request: 1,
        error: CoreError::Unreachable,
    };
    let (s, _) = event(s, failed);
    assert!(s.failure.is_none());
    assert!(s.indexes.queries.loading);
}
