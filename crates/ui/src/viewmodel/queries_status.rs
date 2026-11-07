//! The one-line status of the Queries view (profiler state, poller status, the enable / turn-off
//! button) and what its body shows. Pure and egui-free: the view only draws what is decided here.
//! Every text is an i18n key; no server text is ever carried.
use super::queries::{ProfilingView, QueriesState};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{ProfilerStatus, SamplerPause};
use curlybrace_core::store::PROFILER_MONGOS_MESSAGE;

/// An i18n key with its parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub key: &'static str,
    pub params: Vec<(&'static str, String)>,
}

impl Line {
    fn plain(key: &'static str) -> Self {
        Self {
            key,
            params: Vec::new(),
        }
    }

    fn with(key: &'static str, name: &'static str, value: String) -> Self {
        Self {
            key,
            params: vec![(name, value)],
        }
    }
}

/// The button of the status row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfilerControl {
    Enable,
    TurnOff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusRow {
    pub profiler: Line,
    pub poller: Line,
    /// `None` when there is nothing to switch (state unknown, loading or unavailable).
    pub control: Option<ProfilerControl>,
    pub enabled: bool,
    /// Why the button is off, as an i18n key.
    pub disabled_reason: Option<&'static str>,
}

/// What the area under the chart shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    Loading,
    /// No rows and the profiler is off: explain what enabling does.
    ProfilerOff,
    /// No rows yet and the profiler is on.
    ProfilerOnEmpty,
    /// No rows and the profiler cannot be read.
    Unavailable,
    Rows,
}

pub fn status_row(state: &QueriesState, read_only: bool) -> StatusRow {
    let control = match &state.profiling {
        ProfilingView::Known(profiling) if profiling.level == 0 => Some(ProfilerControl::Enable),
        ProfilingView::Known(_) => Some(ProfilerControl::TurnOff),
        _ => None,
    };
    let disabled_reason = match (read_only, state.busy()) {
        (true, _) => Some("indexes.queries.read_only_note"),
        (false, true) => Some("indexes.queries.busy"),
        (false, false) => None,
    };
    StatusRow {
        profiler: profiler_line(&state.profiling),
        poller: poller_line(&state.profiler_status()),
        control,
        enabled: disabled_reason.is_none(),
        disabled_reason: control.and(disabled_reason),
    }
}

fn profiler_line(profiling: &ProfilingView) -> Line {
    match profiling {
        ProfilingView::Unknown | ProfilingView::Loading => {
            Line::plain("indexes.queries.profiler_checking")
        }
        ProfilingView::Known(p) if p.level == 0 => Line::plain("indexes.queries.profiler_off"),
        ProfilingView::Known(p) => Line::with(
            "indexes.queries.profiler_level",
            "level",
            p.level.to_string(),
        ),
        ProfilingView::Unavailable(error) => Line::plain(unavailable_key(error)),
    }
}

/// The fixed text of a profiler that cannot be read or changed. Only the refusals that tell the
/// person something different are told apart; nothing of the server's text is used.
pub fn unavailable_key(error: &CoreError) -> &'static str {
    match error {
        CoreError::Unauthorized => "indexes.queries.profiler_unauthorized",
        CoreError::Unsupported(text) if text == PROFILER_MONGOS_MESSAGE => {
            "indexes.queries.profiler_mongos"
        }
        _ => "indexes.queries.profiler_unavailable",
    }
}

fn poller_line(status: &ProfilerStatus) -> Line {
    match status {
        ProfilerStatus::Idle => Line::plain("indexes.queries.poller_idle"),
        ProfilerStatus::Reading { databases } => Line::with(
            "indexes.queries.poller_reading",
            "databases",
            databases.to_string(),
        ),
        ProfilerStatus::MissedEntries { db } => {
            Line::with("indexes.queries.poller_missed", "db", db.clone())
        }
        ProfilerStatus::Failing => Line::plain("indexes.queries.poller_failing"),
        ProfilerStatus::Paused(pause) => Line::plain(pause_key(*pause)),
    }
}

fn pause_key(pause: SamplerPause) -> &'static str {
    match pause {
        SamplerPause::NotAuthorized => "indexes.queries.poller_paused_not_authorized",
        SamplerPause::ConnectionLost => "indexes.queries.poller_paused_connection_lost",
        SamplerPause::HistoryUnavailable => "indexes.queries.poller_paused_history_unavailable",
        SamplerPause::HistoryBusy => "indexes.queries.poller_paused_history_busy",
    }
}

/// What to show under the chart given the number of rows.
pub fn body(state: &QueriesState, rows: usize) -> Body {
    match (&state.profiling, rows) {
        (_, n) if n > 0 => Body::Rows,
        (ProfilingView::Known(p), _) if p.level == 0 => Body::ProfilerOff,
        (ProfilingView::Known(_), _) => Body::ProfilerOnEmpty,
        (ProfilingView::Unavailable(_), _) => Body::Unavailable,
        (ProfilingView::Unknown | ProfilingView::Loading, _) => Body::Loading,
    }
}

#[cfg(test)]
#[path = "queries_status_tests.rs"]
mod tests;
