//! View-model of the Queries view of the Indexes tab: the recorded query shapes of the collection,
//! the profiler of its database and the consent step before the profiler is switched.
//! Pure and egui-free; the updaters return new values.
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{VerdictKind, js_string};
use curlybrace_core::history::{ChartRange, ShapeSource, ShapeSummary, is_miss};
use curlybrace_core::runtime::ProfilerStatus;
use curlybrace_core::shape::{Shape, suggest_for_shape};
use curlybrace_core::store::{Namespace, ProfilingState};

/// The profiler level asked for when enabling: every operation is recorded, so fast misses are
/// seen too (and the database carries the overhead).
pub const PROFILER_LEVEL_ON: i32 = 2;
/// The profiler level asked for when disabling.
pub const PROFILER_LEVEL_OFF: i32 = 0;

/// What is known about the profiler of the collection's database.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ProfilingView {
    /// Nothing was asked yet.
    #[default]
    Unknown,
    /// The state was asked for and has not arrived.
    Loading,
    Known(ProfilingState),
    /// The state cannot be read or changed (not authorised, mongos, ...). Shown inline: the shapes
    /// already recorded stay usable.
    Unavailable(CoreError),
}

/// The step before the profiler of `db` is set to `level`: the person sees the exact command and
/// agrees (never silent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfilerConsent {
    pub db: String,
    pub level: i32,
}

impl ProfilerConsent {
    /// The mongosh command the confirmation performs, the database named as an escaped string
    /// literal (the same escaping as the index suggestion), so a name can never change what a
    /// pasted copy of it does.
    pub fn command_text(&self) -> String {
        format!(
            "db.getSiblingDB({}).setProfilingLevel({})",
            js_string(&self.db),
            self.level
        )
    }
}

/// Which view of the Indexes tab is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndexesView {
    #[default]
    Indexes,
    Queries,
}

/// The one profiler command a session has sent and not had answered. The runtime's answers carry
/// no request number, so at most one is ever in flight and the answer is attributed by it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InFlight {
    /// A `ProfilingState` read for this database.
    Read(String),
    /// A `SetProfilingLevel` for this database.
    Write(String),
}

/// What the person did in the Queries view.
#[derive(Clone, Debug, PartialEq)]
pub enum QueriesAction {
    /// Show the view and read it.
    Open,
    Refresh,
    SetRange(ChartRange),
    /// Ask to turn the profiler on (opens the consent step; sends nothing).
    AskEnable,
    ConfirmEnable,
    /// Close the consent step; sends nothing.
    CancelEnable,
    /// Ask to turn the profiler off (opens the consent step; sends nothing).
    AskDisable,
    ConfirmDisable,
}

/// What the Queries view holds for the collection it was opened on.
#[derive(Clone, Debug, PartialEq)]
pub struct QueriesState {
    pub shapes: Vec<ShapeSummary>,
    pub profiling: ProfilingView,
    /// The session's profiler status, once it has reported one; `None` reads as idle.
    pub status: Option<ProfilerStatus>,
    pub consent: Option<ProfilerConsent>,
    /// The number of the newest `QueryShapes` request sent; only its answer is applied. It only
    /// grows, even across collections.
    pub request: u64,
    pub range: ChartRange,
    /// The shapes were requested and have not arrived yet.
    pub loading: bool,
    /// The profiler command sent and not yet answered (see [`InFlight`]); it survives a change of
    /// collection, since its answer is still on its way.
    pub in_flight: Option<InFlight>,
    /// The last shapes read failed. Cleared only by a shapes read that succeeds (or a new request
    /// for them), never by a profiler answer: the two are independent.
    pub shapes_error: Option<CoreError>,
    /// A profiler change whose outcome is unknown. Cleared only by a profiler state that arrives.
    pub profiler_error: Option<CoreError>,
}

impl Default for QueriesState {
    fn default() -> Self {
        Self {
            shapes: Vec::new(),
            profiling: ProfilingView::Unknown,
            status: None,
            consent: None,
            request: 0,
            range: ChartRange::Day,
            loading: false,
            in_flight: None,
            shapes_error: None,
            profiler_error: None,
        }
    }
}

impl QueriesState {
    /// A fresh state for another collection. The request number only grows, the range is the
    /// person's choice, the status belongs to the session, and a profiler change in flight is
    /// still in flight.
    pub fn for_collection(&self) -> Self {
        Self {
            request: self.request,
            range: self.range,
            status: self.status.clone(),
            in_flight: self.in_flight.clone(),
            ..Self::default()
        }
    }

    /// The same state waiting for the shapes under a new request number.
    pub fn requested(self) -> Self {
        Self {
            loading: true,
            shapes_error: None,
            request: self.request + 1,
            ..self
        }
    }

    /// A new range. The shapes on hand were read for the old one, so they are dropped.
    pub fn with_range(self, range: ChartRange) -> Self {
        Self {
            range,
            shapes: Vec::new(),
            ..self
        }
    }

    /// The shapes of the awaited request arrived.
    pub fn with_shapes(self, shapes: Vec<ShapeSummary>) -> Self {
        Self {
            shapes,
            loading: false,
            shapes_error: None,
            ..self
        }
    }

    /// The awaited shapes read failed: shown inline.
    pub fn failed(self, error: CoreError) -> Self {
        Self {
            loading: false,
            shapes_error: Some(error),
            ..self
        }
    }

    pub fn with_profiling(self, profiling: ProfilingView) -> Self {
        Self { profiling, ..self }
    }

    pub fn with_status(self, status: ProfilerStatus) -> Self {
        Self {
            status: Some(status),
            ..self
        }
    }

    pub fn with_consent(self, consent: Option<ProfilerConsent>) -> Self {
        Self { consent, ..self }
    }

    /// The status to show: a poller that has not reported is idle.
    pub fn profiler_status(&self) -> ProfilerStatus {
        self.status.clone().unwrap_or(ProfilerStatus::Idle)
    }

    /// A profiler change for `db` was sent.
    pub fn write_sent(self, db: &str) -> Self {
        Self {
            in_flight: Some(InFlight::Write(db.to_string())),
            ..self
        }
    }

    /// A profiler read for `db` was sent: the state is loading until it is answered.
    pub fn read_sent(self, db: &str) -> Self {
        Self {
            in_flight: Some(InFlight::Read(db.to_string())),
            profiling: ProfilingView::Loading,
            ..self
        }
    }

    /// Whether a profiler change is being sent or answered.
    pub fn writing(&self) -> bool {
        matches!(self.in_flight, Some(InFlight::Write(_)))
    }

    /// Whether the profiler state cannot be asked about or changed right now: a command is in
    /// flight, or the state is still loading.
    pub fn busy(&self) -> bool {
        self.in_flight.is_some() || self.profiling == ProfilingView::Loading
    }

    /// The command in flight was answered.
    pub fn settled(self) -> Self {
        Self {
            in_flight: None,
            ..self
        }
    }
}

/// One shape of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryRow {
    pub shape_json: String,
    pub source: ShapeSource,
    pub executions: u64,
    pub misses: u64,
    pub max_millis: u64,
    /// The worst verdict of the range (shown as secondary text).
    pub worst: VerdictKind,
    /// What the shape does now: the verdict of its newest bucket. Drives the title, the sort and
    /// the suggestion.
    pub latest: VerdictKind,
    /// A ready-to-copy `createIndex` command, for a shape that missed an index only.
    pub suggestion: Option<String>,
}

/// The shapes of `state` as rows, misses first: latest severity, then most misses, then slowest;
/// the shape text last so the order is stable.
pub fn rows(state: &QueriesState, ns: &Namespace) -> Vec<QueryRow> {
    let mut rows: Vec<QueryRow> = state.shapes.iter().map(|s| row_of(s, ns)).collect();
    rows.sort_by(|a, b| {
        b.latest
            .severity()
            .cmp(&a.latest.severity())
            .then(b.misses.cmp(&a.misses))
            .then(b.max_millis.cmp(&a.max_millis))
            .then_with(|| a.shape_json.cmp(&b.shape_json))
    });
    rows
}

fn row_of(summary: &ShapeSummary, ns: &Namespace) -> QueryRow {
    QueryRow {
        shape_json: summary.shape_json.clone(),
        source: summary.source,
        executions: summary.executions,
        misses: summary.misses,
        max_millis: summary.max_millis,
        worst: summary.worst,
        latest: summary.latest,
        suggestion: suggestion_of(summary, ns),
    }
}

/// The advice for a shape that misses an index now; a neutral or good one (an index created since
/// included) needs none, and a shape that cannot be read back is shown without.
fn suggestion_of(summary: &ShapeSummary, ns: &Namespace) -> Option<String> {
    if !is_miss(summary.latest) {
        return None;
    }
    let shape = Shape::from_json(&summary.shape_json).ok()?;
    suggest_for_shape(&shape).map(|s| s.command(ns))
}

#[cfg(test)]
#[path = "queries_tests.rs"]
mod queries_tests;
