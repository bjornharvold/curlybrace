//! Shared helpers of the Queries view tests.
use super::index_write_tests::{browsing, ns, with_flags};
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::{IndexesAction, IndexesState, ProfilerConsent, QueriesAction, SidebarState};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ChartRange, ShapeSource, ShapeSummary};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, ProfilingState};
use uuid::Uuid;
pub(super) const PROFILER_ON: i32 = 2;

pub(super) fn queries(s: AppState, action: QueriesAction) -> (AppState, Vec<Command>) {
    update(s, Input::Indexes(IndexesAction::Queries(action)))
}

pub(super) fn event(s: AppState, ev: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(ev))
}

pub(super) fn failure() -> CoreError {
    CoreError::Database("timed out".into())
}

pub(super) fn level(level: i32) -> ProfilingState {
    ProfilingState {
        level,
        slow_ms: 100,
    }
}

pub(super) fn answer(session: Uuid, db: &str, outcome: Result<ProfilingState, CoreError>) -> Event {
    Event::ProfilingState {
        session,
        db: db.into(),
        outcome,
    }
}

pub(super) fn shape(hash: &str) -> ShapeSummary {
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

pub(super) fn shapes_event(
    session: Uuid,
    range: ChartRange,
    request: u64,
    hashes: &[&str],
) -> Event {
    Event::QueryShapes {
        session,
        ns: ns(),
        range,
        shapes: hashes.iter().map(|h| shape(h)).collect(),
        request,
    }
}

pub(super) fn flags(production: bool, read_only: bool) -> ConnectionConfig {
    with_flags(production, read_only)
}

/// The Queries view opened on `app.people` of a connection with these flags.
pub(super) fn opened(cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = browsing(cfg);
    let (s, _) = queries(s, QueriesAction::Open);
    (s, id)
}

/// Opened, and the profiler state answered: `current` is the level the server reports.
pub(super) fn ready(cfg: ConnectionConfig, current: i32) -> (AppState, Uuid) {
    let (s, id) = opened(cfg);
    let (s, _) = event(s, answer(id, "app", Ok(level(current))));
    (s, id)
}

pub(super) fn asked(cfg: ConnectionConfig, enable: bool) -> (AppState, Uuid) {
    let (s, id) = ready(cfg, if enable { 0 } else { PROFILER_ON });
    let action = match enable {
        true => QueriesAction::AskEnable,
        false => QueriesAction::AskDisable,
    };
    (queries(s, action).0, id)
}

pub(super) fn set_level(session: Uuid, level: i32) -> Command {
    Command::SetProfilingLevel {
        session,
        db: "app".into(),
        level,
    }
}

pub(super) fn view_of() -> SessionView {
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

pub(super) fn consenting(s: AppState, db: &str, level: i32) -> AppState {
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
pub(super) fn switched_to(s: AppState, ns: Namespace) -> AppState {
    let sidebar = s
        .sidebar
        .clone()
        .with_collections(&ns.db, vec![ns.collection.clone()])
        .selecting(ns);
    super::index_actions::collection_opened(AppState { sidebar, ..s }).0
}
