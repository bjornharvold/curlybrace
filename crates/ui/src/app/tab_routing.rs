//! How an answer finds its tab (#159). No route table: an answer belongs to the tab whose own
//! state awaits its number.
//!
//! Every number a tab sends comes from one process-wide source (`request_numbers`), so a number
//! names one request of one tab. A session event with a number goes to the *parked* tab of its
//! session that awaits it; when no parked tab does, it goes to the session's current tab, which
//! is exactly where it went before tabs existed, and that tab's own staleness check drops what is
//! not its own. A closed tab is no longer parked, so it awaits nothing and nothing has to be
//! cleaned up when a tab closes.
//!
//! "Which number does a tab await" is answered in one place: [`Numbers`], built by one
//! constructor from the fields of a tab wherever they live, and [`read_of`], which names the read
//! an event answers.
use super::guard_actions::GuardState;
use super::state::AppState;
use super::tabs::{TabId, TabView};
use crate::viewmodel::aggregate_hint_list::HintList;
use crate::viewmodel::queries::InFlight;
use crate::viewmodel::{AggregateState, IndexesState};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use uuid::Uuid;

/// The numbered reads of a tab: each has the number of its newest request in the tab's own state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Read {
    Find,
    Explain,
    Estimate,
    IndexList,
    IndexHistory,
    QueryShapes,
    Guard,
    Preview,
    AggregateExplain,
    AggregateCount,
    Run,
}

/// The read `ev` answers (or whose failure it is) and the number it echoes. Exhaustive over
/// `Event`: a new event must be classified here.
fn read_of(ev: &Event) -> Option<(Read, u64)> {
    match ev {
        Event::Results { request, .. } => Some((Read::Find, *request)),
        Event::Explain { request, .. } => Some((Read::Explain, *request)),
        Event::EstimatedCount { request, .. } => Some((Read::Estimate, *request)),
        Event::Indexes { request, .. } => Some((Read::IndexList, *request)),
        Event::IndexHistory { request, .. } => Some((Read::IndexHistory, *request)),
        Event::QueryShapes { request, .. } => Some((Read::QueryShapes, *request)),
        Event::GuardChecked { request, .. } => Some((Read::Guard, *request)),
        Event::AggregatePreviewed { request, .. } => Some((Read::Preview, *request)),
        Event::AggregateExplained { request, .. } => Some((Read::AggregateExplain, *request)),
        Event::AggregateCounted { request, .. } => Some((Read::AggregateCount, *request)),
        Event::AggregateRan { run, .. } => Some((Read::Run, *run)),
        Event::RequestFailed {
            context, request, ..
        } => read_of_failure(context).map(|read| (read, *request)),
        // Unnumbered, or numbered for something that is not a tab's (a bulk flow, the
        // statistics, the saved copies).
        Event::Connected { .. }
        | Event::Disconnected { .. }
        | Event::ConnectFailed { .. }
        | Event::Databases { .. }
        | Event::Collections { .. }
        | Event::Mutated { .. }
        | Event::Failed { .. }
        | Event::Compare(_)
        | Event::Copy(_)
        | Event::TestConnection { .. }
        | Event::ProfilingState { .. }
        | Event::IndexChanged { .. }
        | Event::AdminChanged { .. }
        | Event::CollectionStats { .. }
        | Event::DatabaseStats { .. }
        | Event::BulkCount { .. }
        | Event::BulkSample { .. }
        | Event::BulkPreview { .. }
        | Event::BulkFinished { .. }
        | Event::BulkUndoPreview { .. }
        | Event::BulkUndoFinished { .. }
        | Event::Snapshots { .. }
        | Event::BulkCancelResult { .. }
        | Event::Sampler { .. }
        | Event::Profiler { .. }
        | Event::ShellOutput { .. }
        | Event::ShellResult { .. }
        | Event::ShellFinished { .. }
        | Event::ShellRefused { .. }
        | Event::ShellHistory { .. }
        | Event::ShellHistoryChanged { .. } => None,
    }
}

/// The read whose failure carries the `context` of a `RequestFailed`.
fn read_of_failure(context: &str) -> Option<Read> {
    match context {
        "find" => Some(Read::Find),
        "explain_find" => Some(Read::Explain),
        "estimated_count" => Some(Read::Estimate),
        "list_indexes" => Some(Read::IndexList),
        "index_history" => Some(Read::IndexHistory),
        "query_shapes" => Some(Read::QueryShapes),
        "guard_check" => Some(Read::Guard),
        "aggregate_preview" => Some(Read::Preview),
        "aggregate_explain" => Some(Read::AggregateExplain),
        "aggregate_count" => Some(Read::AggregateCount),
        _ => None,
    }
}

/// What one tab awaits: the fields of a tab's state that hold the number of a request in flight,
/// borrowed from wherever the tab lives (the foreground, a session's view or a parked tab).
pub(super) struct Numbers<'a> {
    issued_request: u64,
    verdict_request: u64,
    guard: &'a GuardState,
    indexes: &'a IndexesState,
    aggregate: &'a AggregateState,
}

impl<'a> Numbers<'a> {
    /// The one constructor: every holder of tab fields goes through it.
    pub(super) fn of(
        issued_request: u64,
        verdict_request: u64,
        guard: &'a GuardState,
        indexes: &'a IndexesState,
        aggregate: &'a AggregateState,
    ) -> Self {
        Self {
            issued_request,
            verdict_request,
            guard,
            indexes,
            aggregate,
        }
    }

    fn of_view(view: &'a TabView) -> Self {
        Self::of(
            view.issued_request,
            view.verdict_request,
            &view.guard,
            &view.indexes,
            &view.aggregate,
        )
    }

    fn of_state(state: &'a AppState) -> Self {
        Self::of(
            state.issued_request,
            state.verdict_request,
            &state.guard,
            &state.indexes,
            &state.aggregate,
        )
    }

    fn of_session_view(view: &'a super::session::SessionView) -> Self {
        Self::of(
            view.issued_request,
            view.verdict_request,
            &view.guard,
            &view.indexes,
            &view.aggregate,
        )
    }

    /// Whether the tab awaits `number` for `read`. The one implementation of the question.
    pub(super) fn awaits(&self, read: Read, number: u64) -> bool {
        match read {
            Read::Find => self.issued_request == number,
            Read::Explain => self.verdict_request == number,
            Read::Estimate => {
                self.indexes.estimate_request == number
                    || self.aggregate.run.count_request == number
            }
            Read::IndexList => {
                self.indexes.request == number
                    || matches!(
                        self.aggregate.options_ui.hint.list,
                        HintList::Loading { request } if request == number
                    )
            }
            Read::IndexHistory => self.indexes.request == number,
            Read::QueryShapes => self.indexes.queries.request == number,
            Read::Guard => {
                matches!(self.guard, GuardState::Checking { request, .. } if *request == number)
            }
            Read::Preview => self.aggregate.in_flight == Some(number),
            Read::AggregateExplain => self.aggregate.explain.awaits(number),
            Read::AggregateCount => {
                self.aggregate.counts.awaits(number) || self.aggregate.counts.lingers(number)
            }
            // Only a tab that is running waits for its answer: a copy that kept the number
            // (or a tab whose outcome is unknown) must not take it from the tab that ran.
            Read::Run => self.aggregate.run.run == number && self.aggregate.run.is_running(),
        }
    }

    /// The profiling command in flight, if any.
    fn profiling(&self) -> Option<&InFlight> {
        self.indexes.queries.in_flight.as_ref()
    }
}

/// The numbers of the tab the event will be applied to: the owner (parked), else the session's
/// current tab, wherever its fields live.
fn numbers_of<'a>(state: &'a AppState, session: Uuid, owner: Option<TabId>) -> Option<Numbers<'a>> {
    if let Some(view) = owner.and_then(|tab| state.parked_tabs.get(&tab)) {
        return Some(Numbers::of_view(view));
    }
    match state.active == Some(session) {
        true => Some(Numbers::of_state(state)),
        false => state.parked.get(&session).map(Numbers::of_session_view),
    }
}

/// The parked tab of `session` whose own state awaits `ev`; `None` means the session's current
/// tab takes it, as it did before the tab model.
#[inline(never)]
pub(super) fn owner_of(state: &AppState, session: Uuid, ev: &Event) -> Option<TabId> {
    // The common case: a session with one tab has nothing parked.
    if state.parked_tabs.is_empty() {
        return None;
    }
    if let Event::Failed { context, .. } = ev {
        if super::state::is_tab_write_context(context) {
            return written_from(state, session);
        }
    }
    if profiler_db(ev).is_some() || is_profiling_failure(ev) {
        return profiler_owner(state, session, ev);
    }
    state.tabs.of_session(session).into_iter().find(|tab| {
        state
            .parked_tabs
            .get(tab)
            .is_some_and(|view| view.awaits(ev))
    })
}

/// Who a profiler answer or failure is for. `ProfilingState` does not say whether it answers a
/// read or a change, so the holders of a change (`Write`) claim it first and then the holders of a
/// read; within each, the session's current tab goes before the parked ones. The failure of a
/// command is a read's (a change fails as a write, see `written_from`). `None`: the current tab.
fn profiler_owner(state: &AppState, session: Uuid, ev: &Event) -> Option<TabId> {
    let kinds: &[fn(Option<&InFlight>) -> bool] = match profiler_db(ev) {
        Some(_) => &[is_write, is_read],
        None => &[is_read],
    };
    let db = profiler_db(ev);
    let on_db = |held: Option<&InFlight>| match (held, db) {
        (Some(InFlight::Read(d) | InFlight::Write(d)), Some(db)) => d == db,
        (Some(_), None) => true,
        _ => false,
    };
    for kind in kinds {
        let holds = |held: Option<&InFlight>| kind(held) && on_db(held);
        let current = numbers_of(state, session, None);
        if current.is_some_and(|n| holds(n.profiling())) {
            return None;
        }
        let parked = state.tabs.of_session(session).into_iter().find(|tab| {
            state
                .parked_tabs
                .get(tab)
                .is_some_and(|view| holds(view.indexes.queries.in_flight.as_ref()))
        });
        if parked.is_some() {
            return parked;
        }
    }
    None
}

fn is_write(held: Option<&InFlight>) -> bool {
    matches!(held, Some(InFlight::Write(_)))
}

fn is_read(held: Option<&InFlight>) -> bool {
    matches!(held, Some(InFlight::Read(_)))
}

/// The database a `ProfilingState` answer is for.
fn profiler_db(ev: &Event) -> Option<&str> {
    match ev {
        Event::ProfilingState { db, .. } => Some(db),
        _ => None,
    }
}

/// A profiler read that failed as a command (it names no database).
fn is_profiling_failure(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Failed {
            context: "profiling_state",
            ..
        }
    )
}

/// Whether a tab with `in_flight` is the one a profiler event is for: the answer for its
/// database (a read or a change), or the failure of its read.
fn awaits_profiling(in_flight: Option<&InFlight>, ev: &Event) -> bool {
    match (in_flight, profiler_db(ev)) {
        (Some(InFlight::Read(d) | InFlight::Write(d)), Some(db)) => d == db,
        (Some(InFlight::Read(_)), None) => is_profiling_failure(ev),
        _ => false,
    }
}

/// The event `on_session_event` should see, given the tab that is current while it runs (the
/// owner, when a parked tab awaits the number), or `None` when the event answers a request that
/// tab no longer awaits: a find, explain or estimated count that was closed or superseded is
/// dropped here. A lost server keeps its meaning whoever sent the request. The failures of the
/// three reads are turned back into the bare `Failed` they were: converted here, once, ahead of
/// `on_session_event`, instead of re-dispatched from inside it, because that frame is the one a
/// lost connection recurses through and one more level of it costs hundreds of KiB of debug stack.
pub(super) fn check(
    state: &AppState,
    session: Uuid,
    owner: Option<TabId>,
    ev: Event,
) -> Option<Event> {
    let (read, number) = match (&ev, read_of(&ev)) {
        (
            Event::Explain { .. } | Event::EstimatedCount { .. } | Event::RequestFailed { .. },
            Some((read @ (Read::Find | Read::Explain | Read::Estimate), number)),
        ) => (read, number),
        _ => return Some(ev),
    };
    let lost = matches!(
        &ev,
        Event::RequestFailed {
            error: CoreError::Unreachable | CoreError::TlsHandshake,
            ..
        }
    );
    let awaited = numbers_of(state, session, owner).is_some_and(|n| n.awaits(read, number));
    match (ev, lost || awaited) {
        (_, false) => None,
        (
            Event::RequestFailed {
                session,
                context,
                error,
                ..
            },
            true,
        ) => Some(Event::Failed {
            session,
            context,
            error,
        }),
        (other, true) => Some(other),
    }
}

/// The Aggregate draft `ev`'s request belongs to: that of the tab `owner` names (a parked tab which
/// awaits it), else that of the session's current tab (in the foreground when the session is
/// active, else in its parked view). The lost-server checks of the preview, explain and count
/// answers read it before the answer is applied, so they must look where the answer will.
pub(super) fn draft_for(
    state: &AppState,
    session: Uuid,
    owner: Option<TabId>,
) -> Option<&AggregateState> {
    if let Some(tab) = owner {
        return state.parked_tabs.get(&tab).map(|view| &*view.aggregate);
    }
    match state.active == Some(session) {
        true => Some(&*state.aggregate),
        false => state.parked.get(&session).map(|v| &*v.aggregate),
    }
}

/// The parked tab of `session` the write in flight was sent from. The write is read from the
/// event's own session: the foreground's when it is the active one, else its parked view's.
fn written_from(state: &AppState, session: Uuid) -> Option<TabId> {
    let pending = match state.active == Some(session) {
        true => state.pending.as_ref(),
        false => state.parked.get(&session).and_then(|v| v.pending.as_ref()),
    };
    let tab = pending.and_then(|p| p.tab)?;
    let ours = state.tabs.owner.get(&tab) == Some(&session);
    (ours && state.parked_tabs.contains_key(&tab)).then_some(tab)
}

impl AppState {
    /// `with_session`, then in the tab `owner` names (see [`owner_of`]) when there is one.
    #[inline(always)]
    pub(super) fn with_session_tab(
        self,
        session: Uuid,
        owner: Option<TabId>,
        f: impl FnOnce(AppState) -> super::reducer::Out,
    ) -> super::reducer::Out {
        match owner {
            Some(tab) => self.with_tab(tab, f),
            None => self.with_session(session, f),
        }
    }
}

impl TabView {
    /// Whether this tab's own state awaits `ev`: it carries the number of a request this tab
    /// sent and has not given up on. A number of `0` is never a request.
    pub fn awaits(&self, ev: &Event) -> bool {
        let numbers = Numbers::of_view(self);
        if let Some((read, number)) = read_of(ev) {
            let pending_verdict = !matches!(ev, Event::Explain { .. })
                || matches!(self.verdict, super::VerdictState::Pending { .. });
            return number != 0 && pending_verdict && numbers.awaits(read, number);
        }
        // Not numbered: a profiler answer or a read that failed as a command.
        awaits_profiling(numbers.profiling(), ev)
    }
}

#[cfg(test)]
#[path = "tab_routing_aggregate_tests.rs"]
mod aggregate_tests;
#[cfg(test)]
#[path = "tab_background_tests.rs"]
mod background_tests;
#[cfg(test)]
#[path = "tab_claims_tests.rs"]
mod claims_tests;
#[cfg(test)]
#[path = "tab_routing_events_tests.rs"]
mod events_tests;
#[cfg(test)]
#[path = "tab_followup_tests.rs"]
mod followup_tests;
#[cfg(test)]
#[path = "tab_single_tab_tests.rs"]
mod single_tab_tests;
#[cfg(test)]
#[path = "tab_stack_tests.rs"]
mod stack_tests;
#[cfg(test)]
#[path = "tab_routing_tests.rs"]
mod tests;
