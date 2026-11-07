//! The Queries view in the reducer: single-flight profiler commands and failures that stay inline.
use super::index_write_tests::shown;
use super::queries_test_support::*;
use super::reducer::{Input, update};
use super::state::AppState;
use super::write_guard::WriteWarningAction;
use crate::viewmodel::{ProfilerConsent, ProfilingView, QueriesAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
const PROFILER_ON: i32 = 2;

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
