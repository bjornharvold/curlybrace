//! A profiler command sent from a parked tab is settled in that tab (#159), and one blip does not
//! discard an editor that sits in a tab which neither owns the answer nor is in view.
use crate::app::reducer::{Input, update};
use crate::app::state::is_tab_write_context;
use crate::app::state::{AppState, ConnStatus};
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{act, ev, orders, parked, two_tabs};
use crate::viewmodel::{CollectionTab, DocAction, IndexesAction, QueriesAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::ProfilingState;
use uuid::Uuid;

fn queries(s: AppState, tab: TabId, action: QueriesAction) -> (AppState, Vec<Command>) {
    act(s, tab, Input::Indexes(IndexesAction::Queries(action)))
}

/// Tab two (parked) opened its Queries view and its profiling read is in flight.
fn reading_in_tab_two() -> (AppState, Uuid, TabId) {
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Indexes));
    let (s, commands) = queries(s, t.second, QueriesAction::Open);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ProfilingState { .. }))
    );
    (s, t.a, t.second)
}

/// Tab two changed the profiling level: the write is in flight and `pending` names the tab.
fn changing_in_tab_two() -> (AppState, Uuid, TabId) {
    let (s, a, second) = reading_in_tab_two();
    let level = ProfilingState {
        level: 0,
        slow_ms: 100,
    };
    let s = ev(
        s,
        Event::ProfilingState {
            session: a,
            db: "app".into(),
            outcome: Ok(level),
        },
    );
    let (s, _) = queries(s, second, QueriesAction::AskEnable);
    let (s, commands) = queries(s, second, QueriesAction::ConfirmEnable);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::SetProfilingLevel { .. })),
        "{commands:?}"
    );
    assert_eq!(s.pending.as_ref().and_then(|p| p.tab), Some(second));
    (s, a, second)
}

fn writing(s: &AppState, tab: TabId) -> bool {
    use crate::viewmodel::queries::InFlight;
    matches!(
        parked(s, tab).indexes.queries.in_flight,
        Some(InFlight::Write(_))
    )
}

fn command_failed(session: Uuid, context: &'static str, error: CoreError) -> Event {
    Event::Failed {
        session,
        context,
        error,
    }
}

#[test]
fn a_refused_profiler_change_of_a_parked_tab_frees_the_write() {
    let (s, a, second) = changing_in_tab_two();
    let s = ev(
        s,
        command_failed(a, "set_profiling_level", CoreError::Database("no".into())),
    );
    assert!(s.pending.is_none(), "a later write is not refused for it");
    // The change is over; the view reads the level back once, as for any failed change.
    assert!(!writing(&s, second), "the view is not stuck on the write");
}

#[test]
fn a_lost_server_on_a_parked_profiler_change_frees_the_write() {
    let (mut s, a, second) = changing_in_tab_two();
    // An editor keeps the session through the blip, so what is left to settle is the command.
    let (s2, _) = update(std::mem::take(&mut s), Input::Doc(DocAction::Insert));
    let s = ev(
        s2,
        command_failed(a, "set_profiling_level", CoreError::Unreachable),
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.pending.is_none());
    assert!(!writing(&s, second));
}

#[test]
fn a_failed_profiling_read_of_a_parked_tab_settles_there() {
    let (s, a, second) = reading_in_tab_two();
    assert!(parked(&s, second).indexes.queries.in_flight.is_some());
    let s = ev(
        s,
        command_failed(a, "profiling_state", CoreError::Database("no".into())),
    );
    assert!(parked(&s, second).indexes.queries.in_flight.is_none());
}

// The tab that holds the editor neither owns the answer nor is in view.

#[test]
fn a_lost_server_keeps_the_session_for_an_editor_in_a_third_tab() {
    let t = two_tabs();
    let (s, third) = s_with_third(t.s, t.a);
    let (s, _) = act(s, third, Input::Doc(DocAction::Insert));
    assert!(s.editor.is_none() && parked(&s, t.second).editor.is_none());
    let s = ev(
        s,
        Event::RequestFailed {
            session: t.a,
            context: "find",
            request: t.second_find,
            error: CoreError::Unreachable,
        },
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(
        parked(&s, third).editor.is_some(),
        "the typed text survives"
    );
}

fn s_with_third(s: AppState, a: Uuid) -> (AppState, TabId) {
    let (s, third) = s.tab_opened(a);
    let third = third.expect("a third tab");
    let (s, _) = act(
        s,
        third,
        Input::Sidebar(crate::viewmodel::SidebarAction::Open(orders())),
    );
    (s, third)
}

// One list of the failures a tab's write can have.

#[test]
fn every_command_a_tab_writes_names_its_failure_in_the_shared_list() {
    use curlybrace_core::indexes::IndexSpec;
    let (session, ns) = (Uuid::nil(), orders());
    let commands = [
        Command::Insert {
            session,
            ns: ns.clone(),
            json: String::new(),
        },
        Command::Replace {
            session,
            ns: ns.clone(),
            id: bson::Bson::Int32(1),
            json: String::new(),
        },
        Command::Delete {
            session,
            ns: ns.clone(),
            id: bson::Bson::Int32(1),
        },
        Command::Clone {
            session,
            ns: ns.clone(),
            source: bson::Document::new(),
        },
        Command::CreateIndex {
            session,
            ns: ns.clone(),
            spec: IndexSpec {
                name: "a_1".into(),
                keys: Vec::new(),
                options: Default::default(),
                raw: bson::Document::new(),
            },
        },
        Command::DropIndex {
            session,
            ns: ns.clone(),
            name: String::new(),
        },
        Command::SetIndexHidden {
            session,
            ns: ns.clone(),
            name: String::new(),
            hidden: true,
        },
        Command::SetIndexTtl {
            session,
            ns: ns.clone(),
            name: String::new(),
            seconds: 1,
        },
        Command::SetProfilingLevel {
            session,
            db: "app".into(),
            level: 1,
        },
        Command::AggregateRun {
            session,
            ns,
            pipeline: Vec::new(),
            run: 1,
            options: Default::default(),
        },
    ];
    for command in commands {
        assert!(command.is_write(), "{command:?}");
        assert!(
            is_tab_write_context(command.context()),
            "{}",
            command.context()
        );
    }
}
