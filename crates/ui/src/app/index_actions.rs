//! Pure handlers for the Indexes tab: what the person does on it and what the runtime answers.
//! Requests are only produced while connected, like every other data action.
use super::actions::gated;
use super::index_edit_actions as edit;
use super::index_write_actions as write;
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::{CollectionTab, IndexesAction};
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::{Command, IndexSeries, SamplerStatus};
use curlybrace_core::store::{CollectionIndex, Namespace};

pub(super) fn on_action(state: AppState, action: IndexesAction) -> Out {
    match action {
        IndexesAction::OpenTab => gated(state, open_tab),
        IndexesAction::Refresh => gated(state, request),
        IndexesAction::SetRange(range) => gated(state, |s| {
            let indexes = s.indexes.with_range(range);
            request(AppState { indexes, ..s })
        }),
        IndexesAction::Select(name) => {
            let indexes = state.indexes.selecting(name);
            (AppState { indexes, ..state }, Vec::new())
        }
        // Nothing to record: the copy itself is a clipboard action wired with the detail view.
        IndexesAction::CopyJson(_) => (state, Vec::new()),
        IndexesAction::OpenBuilder => write::open_builder(state),
        IndexesAction::CancelBuilder => write::cancel_builder(state),
        IndexesAction::Builder(edit) => write::edit_builder(state, edit),
        IndexesAction::Create => write::create(state),
        IndexesAction::EditCopy(name) => edit::edit_copy(state, name),
        IndexesAction::ConfirmDropOld => edit::confirm_drop_old(state),
        IndexesAction::KeepOld => edit::keep_old(state),
        IndexesAction::SetHidden(name, hidden) => edit::set_hidden(state, name, hidden),
        IndexesAction::ChangeTtl(name) => edit::change_ttl(state, name),
        IndexesAction::TtlTyped(text) => edit::ttl_typed(state, text),
        IndexesAction::TtlCancelled => edit::ttl_cancelled(state),
        IndexesAction::SetTtl(name, seconds) => edit::set_ttl(state, name, seconds),
        IndexesAction::DropRequested(name) => edit::drop_requested(state, name),
        IndexesAction::DropTyped(text) => edit::drop_typed(state, text),
        IndexesAction::DropConfirmed => edit::drop_confirmed(state),
        IndexesAction::DropCancelled => edit::drop_cancelled(state),
    }
}

/// Shows the tab `tab`; the Indexes tab behaves like `OpenTab`, so while not connected it is
/// refused with the banner and the tab stays where it was.
pub(super) fn switch_tab(state: AppState, tab: CollectionTab) -> Out {
    match tab {
        CollectionTab::Documents => (AppState { tab, ..state }, Vec::new()),
        CollectionTab::Indexes => gated(state, open_tab),
    }
}

fn open_tab(state: AppState) -> Out {
    request(AppState {
        tab: CollectionTab::Indexes,
        ..state
    })
}

/// Asks for the indexes and their usage of the selected collection over the current range. A
/// collection other than the one the tab holds starts over; the same one keeps showing what it
/// has until the answers replace it.
fn request(state: AppState) -> Out {
    let Some(ns) = state.selected_ns().cloned() else {
        return (state, Vec::new());
    };
    let session = state.session_id();
    let range = state.indexes.range;
    let held = state.indexes.ns.as_ref() == Some(&ns);
    let indexes = match held {
        true => state.indexes,
        false => state.indexes.for_collection(ns.clone()),
    };
    let commands = vec![
        Command::ListIndexes {
            session,
            ns: ns.clone(),
        },
        Command::IndexHistory { session, ns, range },
    ];
    let indexes = indexes.requested();
    (AppState { indexes, ..state }, commands)
}

/// A collection was just opened in the sidebar (`state.sidebar` already selects it): the Indexes
/// state restarts for it, and when the Indexes tab is the one shown its lists are requested.
/// The documents tab requests nothing; opening the tab later does.
pub(super) fn collection_opened(state: AppState) -> (AppState, Vec<Command>) {
    let Some(ns) = state.selected_ns().cloned() else {
        return (state, Vec::new());
    };
    let indexes = state.indexes.for_collection(ns);
    let next = AppState { indexes, ..state };
    match next.tab {
        CollectionTab::Indexes => request(next),
        CollectionTab::Documents => (next, Vec::new()),
    }
}

/// The answer to `ListIndexes`; one for another collection is stale.
pub(super) fn indexes_listed(
    state: AppState,
    ns: Namespace,
    indexes: Vec<CollectionIndex>,
) -> AppState {
    if state.indexes.ns.as_ref() != Some(&ns) {
        return state;
    }
    let indexes = state.indexes.with_indexes(indexes);
    AppState { indexes, ..state }
}

/// The answer to `IndexHistory`; one for another collection or range is stale.
pub(super) fn history_read(
    state: AppState,
    ns: Namespace,
    range: ChartRange,
    series: Vec<IndexSeries>,
) -> AppState {
    if state.indexes.ns.as_ref() != Some(&ns) || state.indexes.range != range {
        return state;
    }
    let indexes = state.indexes.with_series(series);
    AppState { indexes, ..state }
}

/// An index write succeeded: the write in flight is settled, then the collection on screen is
/// listed again, any other is left alone.
pub(super) fn changed(state: AppState, ns: Namespace) -> Out {
    let state = write::write_finished(state, &ns);
    match state.indexes.ns.as_ref() == Some(&ns) && state.selected_ns() == Some(&ns) {
        true => request(state),
        false => (state, Vec::new()),
    }
}

/// The session's sampler started, resumed or paused. Runs while the event's own session is the
/// active one (see `AppState::with_session`), so the status lands in that session's view.
pub(super) fn sampler_reported(state: AppState, status: SamplerStatus) -> AppState {
    let indexes = state.indexes.with_sampler(status);
    AppState { indexes, ..state }
}
