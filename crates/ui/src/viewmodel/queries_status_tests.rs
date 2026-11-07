//! The one-line status of the Queries view and its body.
use super::*;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::runtime::{ProfilerStatus, SamplerPause};
use curlybrace_core::store::{PROFILER_MONGOS_MESSAGE, ProfilingState};

fn known(level: i32) -> QueriesState {
    QueriesState::default().with_profiling(ProfilingView::Known(ProfilingState {
        level,
        slow_ms: 100,
    }))
}

#[test]
fn the_button_follows_the_level() {
    assert_eq!(
        status_row(&known(0), false).control,
        Some(ProfilerControl::Enable)
    );
    assert_eq!(
        status_row(&known(2), false).control,
        Some(ProfilerControl::TurnOff)
    );
    for state in [
        QueriesState::default(),
        QueriesState::default().with_profiling(ProfilingView::Loading),
        QueriesState::default().with_profiling(ProfilingView::Unavailable(CoreError::Unauthorized)),
    ] {
        assert_eq!(status_row(&state, false).control, None);
    }
}

#[test]
fn the_button_is_off_while_busy_or_read_only_and_says_why() {
    let row = status_row(&known(0), false);
    assert!(row.enabled && row.disabled_reason.is_none());
    let read_only = status_row(&known(0), true);
    assert!(!read_only.enabled);
    assert_eq!(
        read_only.disabled_reason,
        Some("indexes.queries.read_only_note")
    );
    let busy = status_row(&known(0).write_sent("shop"), false);
    assert!(!busy.enabled);
    assert_eq!(busy.disabled_reason, Some("indexes.queries.busy"));
}

#[test]
fn unavailable_reasons_never_carry_server_text() {
    let secret = "secret-detail";
    for (error, key) in [
        (
            CoreError::Unauthorized,
            "indexes.queries.profiler_unauthorized",
        ),
        (
            CoreError::Unsupported(PROFILER_MONGOS_MESSAGE.into()),
            "indexes.queries.profiler_mongos",
        ),
        (
            CoreError::Unsupported(secret.into()),
            "indexes.queries.profiler_unavailable",
        ),
        (
            CoreError::Database(secret.into()),
            "indexes.queries.profiler_unavailable",
        ),
    ] {
        let state = QueriesState::default().with_profiling(ProfilingView::Unavailable(error));
        let row = status_row(&state, false);
        assert_eq!(row.profiler.key, key);
        assert!(row.profiler.params.iter().all(|(_, v)| v != secret));
    }
}

#[test]
fn the_poller_lines_use_their_own_keys() {
    let line = |s: ProfilerStatus| status_row(&known(2).with_status(s), false).poller.key;
    assert_eq!(
        status_row(&known(2), false).poller.key,
        "indexes.queries.poller_idle"
    );
    assert_eq!(
        line(ProfilerStatus::Reading { databases: 2 }),
        "indexes.queries.poller_reading"
    );
    assert_eq!(
        line(ProfilerStatus::MissedEntries { db: "shop".into() }),
        "indexes.queries.poller_missed"
    );
    assert_eq!(
        line(ProfilerStatus::Failing),
        "indexes.queries.poller_failing"
    );
    let pauses = [
        SamplerPause::NotAuthorized,
        SamplerPause::ConnectionLost,
        SamplerPause::HistoryUnavailable,
        SamplerPause::HistoryBusy,
    ]
    .map(|p| line(ProfilerStatus::Paused(p)));
    let mut unique = pauses.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 4);
}

#[test]
fn the_body_depends_on_the_profiler_when_there_are_no_rows() {
    assert_eq!(body(&known(0), 0), Body::ProfilerOff);
    assert_eq!(body(&known(2), 0), Body::ProfilerOnEmpty);
    assert_eq!(body(&QueriesState::default(), 0), Body::Loading);
    let off =
        QueriesState::default().with_profiling(ProfilingView::Unavailable(CoreError::NotFound));
    assert_eq!(body(&off, 0), Body::Unavailable);
    assert_eq!(body(&known(0), 3), Body::Rows);
}

#[test]
fn every_verdict_kind_has_an_icon_and_a_title() {
    for kind in [
        VerdictKind::IndexUsed,
        VerdictKind::CollectionScan,
        VerdictKind::NoFilter,
    ] {
        let (icon, key) = crate::viewmodel::verdict::kind_presentation(kind);
        assert!(!icon.is_empty() && key.starts_with("query.verdict.title_"));
    }
}
