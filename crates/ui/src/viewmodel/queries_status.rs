//! The one-line status of the Queries view (profiler state, poller status, the enable / turn-off
//! button) and what its body shows. Pure and egui-free: the view only draws what is decided here.
//! Every text is an i18n key; no server text is ever carried.
use super::queries::{ProfilingView, QueriesState};
use super::verdict::kind_presentation;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::VerdictKind;
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
    /// Level 1 records only slow operations: ask for level 2 (every operation) instead.
    RecordAll,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusRow {
    pub profiler: Line,
    pub poller: Line,
    /// Empty when there is nothing to switch (state unknown, loading or unavailable).
    pub controls: Vec<ProfilerControl>,
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
    let controls = match &state.profiling {
        ProfilingView::Known(p) if p.level == 0 => vec![ProfilerControl::Enable],
        ProfilingView::Known(p) if p.level == 1 => {
            vec![ProfilerControl::TurnOff, ProfilerControl::RecordAll]
        }
        ProfilingView::Known(_) => vec![ProfilerControl::TurnOff],
        _ => Vec::new(),
    };
    let disabled_reason = match (read_only, state.busy()) {
        (true, _) => Some("indexes.queries.read_only_note"),
        (false, true) => Some("indexes.queries.busy"),
        (false, false) => None,
    };
    StatusRow {
        profiler: profiler_line(&state.profiling),
        poller: poller_line(&state.profiler_status()),
        enabled: disabled_reason.is_none(),
        disabled_reason: disabled_reason.filter(|_| !controls.is_empty()),
        controls,
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
        ProfilerStatus::Reading { databases } => {
            let key = match databases {
                1 => "indexes.queries.poller_reading_one",
                _ => "indexes.queries.poller_reading_other",
            };
            Line::with(key, "databases", databases.to_string())
        }
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

/// The note for level 1, which records only operations slower than `slow_ms`: a fast query that
/// misses an index is never captured, and the person should know before trusting an empty list.
pub fn level_note(state: &QueriesState) -> Option<Line> {
    match &state.profiling {
        ProfilingView::Known(p) if p.level == 1 => Some(Line::with(
            "indexes.queries.level_one_note",
            "slow_ms",
            p.slow_ms.to_string(),
        )),
        _ => None,
    }
}

/// The icon and title key of a row: the verdict's own, except that the neutral kinds read as a
/// list of app queries ("ran against an empty collection"), not as a verdict on the collection.
pub fn row_title(kind: VerdictKind) -> (&'static str, &'static str) {
    let (icon, key) = kind_presentation(kind);
    let key = match kind {
        VerdictKind::EmptyCollection => "indexes.queries.title_empty",
        VerdictKind::NoFilter => "indexes.queries.title_no_filter",
        _ => key,
    };
    (icon, key)
}

/// The run count's key: the language has one form for exactly 1 and one for the rest.
pub fn runs_key(n: u64) -> &'static str {
    match n {
        1 => "indexes.queries.runs_one",
        _ => "indexes.queries.runs_other",
    }
}

pub fn misses_key(n: u64) -> &'static str {
    match n {
        1 => "indexes.queries.misses_one",
        _ => "indexes.queries.misses_other",
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
