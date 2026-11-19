//! How an answer finds its tab (#159). No route table: an answer belongs to the tab whose own
//! state awaits its number.
//!
//! Every number a tab sends comes from one process-wide source (`request_numbers`), so a number
//! names one request of one tab. A session event with a number goes to the *parked* tab of its
//! session that awaits it; when no parked tab does, it goes to the session's current tab, which
//! is exactly where it went before tabs existed, and that tab's own staleness check drops what is
//! not its own. A closed tab is no longer parked, so it awaits nothing and nothing has to be
//! cleaned up when a tab closes.
use super::guard_actions::GuardState;
use super::state::AppState;
use super::tabs::{TabId, TabView};
use crate::viewmodel::AggregateState;
use crate::viewmodel::aggregate_hint_list::HintList;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use uuid::Uuid;

/// The parked tab of `session` whose own state awaits `ev`; `None` means the session's current
/// tab takes it, as it did before the tab model.
#[inline(never)]
pub(super) fn owner_of(state: &AppState, session: Uuid, ev: &Event) -> Option<TabId> {
    // The common case: a session with one tab has nothing parked.
    if state.parked_tabs.is_empty() {
        return None;
    }
    if let Event::Failed { context, .. } = ev {
        if WRITE_CONTEXTS.contains(context) {
            return written_from(state, session);
        }
    }
    state.tabs.of_session(session).into_iter().find(|tab| {
        state
            .parked_tabs
            .get(tab)
            .is_some_and(|view| view.awaits(ev))
    })
}

/// The event `on_session_event` should see, given the tab that is current while it runs (the
/// owner, when a parked tab awaits the number). An answer or failure of a find, explain or
/// estimated count whose number this tab does not await is the answer of a closed or superseded
/// request: it becomes a superseded failure, which `on_session_event` consumes. A lost server
/// keeps its meaning whoever sent the request. The failures of the three reads are then
/// turned back into the bare `Failed` they were (see [`unnumbered`]).
pub(super) fn check(state: &AppState, session: Uuid, owner: Option<TabId>, ev: &mut Event) {
    let numbers = numbers_of(state, session, owner);
    let stale = match &*ev {
        Event::RequestFailed {
            context: context @ ("find" | "explain_find" | "estimated_count"),
            request,
            error,
            ..
        } => {
            let lost = matches!(error, CoreError::Unreachable | CoreError::TlsHandshake);
            !lost && !numbers.awaits(context, *request)
        }
        Event::Explain { request, .. } => !numbers.awaits("explain_find", *request),
        Event::EstimatedCount { request, .. } => !numbers.awaits("estimated_count", *request),
        _ => false,
    };
    // Replaced in place: this runs on the deepest event path, where a second `Event` costs stack.
    if let (true, Some(session)) = (stale, ev.session()) {
        *ev = Event::Failed {
            session,
            context: "find",
            error: CoreError::Superseded,
        };
    } else if matches!(ev, Event::RequestFailed { .. }) {
        let taken = std::mem::replace(
            ev,
            Event::Failed {
                session: Uuid::nil(),
                context: "find",
                error: CoreError::Superseded,
            },
        );
        *ev = unnumbered(taken);
    }
}

/// The numbers of the reads one tab has sent and not given up on.
struct Numbers {
    find: u64,
    explain: u64,
    estimate: u64,
    count: u64,
}

impl Numbers {
    /// Whether the read `context` names awaits `request`.
    fn awaits(&self, context: &str, request: u64) -> bool {
        match context {
            "find" => self.find == request,
            "explain_find" => self.explain == request,
            _ => self.estimate == request || self.count == request,
        }
    }
}

/// The numbers of the tab the event will be applied to: the owner (parked), else the session's
/// current tab, wherever its fields live.
fn numbers_of(state: &AppState, session: Uuid, owner: Option<TabId>) -> Numbers {
    if let Some(view) = owner.and_then(|tab| state.parked_tabs.get(&tab)) {
        return Numbers {
            find: view.issued_request,
            explain: view.verdict_request,
            estimate: view.indexes.estimate_request,
            count: view.aggregate.run.count_request,
        };
    }
    match state.active == Some(session) {
        true => Numbers {
            find: state.issued_request,
            explain: state.verdict_request,
            estimate: state.indexes.estimate_request,
            count: state.aggregate.run.count_request,
        },
        false => state.parked.get(&session).map_or(
            Numbers {
                find: 0,
                explain: 0,
                estimate: 0,
                count: 0,
            },
            |v| Numbers {
                find: v.issued_request,
                explain: v.verdict_request,
                estimate: v.indexes.estimate_request,
                count: v.aggregate.run.count_request,
            },
        ),
    }
}

/// A find, an explain and an estimated count are numbered reads (the number routes their answers),
/// but their failures are handled as the bare `Failed` they were before. Converted here, once, ahead
/// of `on_session_event`, instead of re-dispatched from inside it: that frame is the one a lost
/// connection recurses through, and one more level of it costs hundreds of KiB of debug-build stack.
fn unnumbered(ev: Event) -> Event {
    match ev {
        Event::RequestFailed {
            session,
            context: context @ ("find" | "explain_find" | "estimated_count"),
            error,
            ..
        } => Event::Failed {
            session,
            context,
            error,
        },
        other => other,
    }
}

/// The Aggregate draft `ev`'s request belongs to: that of the parked tab which awaits it, else that
/// of the session's current tab (in the foreground when the session is active, else in its parked
/// view). The lost-server checks of the preview, explain and count answers read it before the
/// answer is applied, so they must look where the answer will.
pub(super) fn draft_for<'a>(
    state: &'a AppState,
    session: Uuid,
    ev: &Event,
) -> Option<&'a AggregateState> {
    if let Some(tab) = owner_of(state, session, ev) {
        return state.parked_tabs.get(&tab).map(|view| &*view.aggregate);
    }
    match state.active == Some(session) {
        true => Some(&*state.aggregate),
        false => state.parked.get(&session).map(|v| &*v.aggregate),
    }
}

/// The commands whose failure belongs to the tab the write was sent from (see `Pending::tab`).
const WRITE_CONTEXTS: [&str; 9] = [
    "insert",
    "replace",
    "delete",
    "clone",
    "create_index",
    "drop_index",
    "set_index_hidden",
    "set_index_ttl",
    "aggregate_run",
];

/// The parked tab of `session` the write in flight was sent from.
fn written_from(state: &AppState, session: Uuid) -> Option<TabId> {
    let tab = state.pending.as_ref().and_then(|p| p.tab)?;
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

fn guard_awaits(guard: &GuardState, request: u64) -> bool {
    matches!(guard, GuardState::Checking { request: r, .. } if *r == request)
}

impl TabView {
    /// Whether this tab's own state awaits `ev`: it carries the number of a request this tab
    /// sent and has not given up on. Exhaustive over `Event`, so a new event must be classified.
    pub fn awaits(&self, ev: &Event) -> bool {
        match ev {
            Event::Results { request, .. } => self.issued_request == *request && *request != 0,
            Event::Explain { request, .. } => {
                self.verdict_request == *request
                    && *request != 0
                    && matches!(self.verdict, super::VerdictState::Pending { .. })
            }
            Event::EstimatedCount { request, .. } => self.awaits_estimate(*request),
            Event::Indexes { request, .. } => self.awaits_index_list(*request),
            Event::IndexHistory { request, .. } => {
                self.indexes.request == *request && *request != 0
            }
            Event::QueryShapes { request, .. } => {
                self.indexes.queries.request == *request && *request != 0
            }
            Event::GuardChecked { request, .. } => guard_awaits(&self.guard, *request),
            Event::AggregatePreviewed { request, .. } => self.awaits_preview(*request),
            Event::AggregateExplained { request, .. } => self.aggregate.explain.awaits(*request),
            Event::AggregateCounted { request, .. } => {
                self.aggregate.counts.awaits(*request) || self.aggregate.counts.lingers(*request)
            }
            Event::AggregateRan { run, .. } => self.aggregate.run.run == *run && *run != 0,
            Event::RequestFailed {
                context, request, ..
            } => self.awaits_failure(context, *request),
            // Not numbered: the answer to the profiler read or change this tab has in flight for
            // the database.
            Event::ProfilingState { db, .. } => self.awaits_profiling(db),
            // Unnumbered, or numbered for something that is not a tab's (a bulk flow, the
            // statistics, the saved copies): the session's current tab takes them.
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
            | Event::ShellHistoryChanged { .. } => false,
        }
    }

    /// This tab asked for the profiling state of `db` (or changed it) and has no answer yet.
    fn awaits_profiling(&self, db: &str) -> bool {
        use crate::viewmodel::queries::InFlight;
        matches!(
            &self.indexes.queries.in_flight,
            Some(InFlight::Read(d) | InFlight::Write(d)) if d == db
        )
    }

    /// The builder's or the run dialog's count of this tab.
    fn awaits_estimate(&self, request: u64) -> bool {
        request != 0
            && (self.indexes.estimate_request == request
                || self.aggregate.run.count_request == request)
    }

    /// The index list of the Indexes tab, or the hint picker's read (numbered above the base).
    fn awaits_index_list(&self, request: u64) -> bool {
        request != 0
            && (self.indexes.request == request
                || matches!(
                    self.aggregate.options_ui.hint.list,
                    HintList::Loading { request: r } if r == request
                ))
    }

    /// The preview in flight of this tab's draft: at most one per draft.
    fn awaits_preview(&self, request: u64) -> bool {
        request != 0 && self.aggregate.in_flight == Some(request)
    }

    /// The failure of the numbered read `context` names.
    fn awaits_failure(&self, context: &str, request: u64) -> bool {
        match context {
            "find" => self.issued_request == request && request != 0,
            "explain_find" => self.verdict_request == request && request != 0,
            "estimated_count" => self.awaits_estimate(request),
            "list_indexes" => self.awaits_index_list(request),
            "index_history" => self.indexes.request == request && request != 0,
            "query_shapes" => self.indexes.queries.request == request && request != 0,
            "guard_check" => guard_awaits(&self.guard, request),
            "aggregate_preview" => self.awaits_preview(request),
            "aggregate_explain" => self.aggregate.explain.awaits(request),
            "aggregate_count" => {
                self.aggregate.counts.awaits(request) || self.aggregate.counts.lingers(request)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "tab_routing_aggregate_tests.rs"]
mod aggregate_tests;
#[cfg(test)]
#[path = "tab_routing_events_tests.rs"]
mod events_tests;
#[cfg(test)]
#[path = "tab_stack_tests.rs"]
mod stack_tests;
#[cfg(test)]
#[path = "tab_routing_tests.rs"]
mod tests;
