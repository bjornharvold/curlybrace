//! Reducer arms of the index builder: opening it, editing it and creating the index through the
//! same write guard as every other write, and what the runtime answers to that.
use super::actions::{gated, mutation};
use super::reducer::Out;
use super::state::{AppState, Banner};
use super::write_guard::{Guard, WriteKind};
use crate::viewmodel::aggregate_hint_list::HintListUi;
use crate::viewmodel::{BuilderEdit, IndexBuilderState, IndexWrite};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;

/// Opens the builder on the selected collection and asks for its document count, which only
/// decides whether the large-collection warning shows. A read-only connection never gets one.
pub(super) fn open_builder(state: AppState) -> Out {
    open_with(state, |_, ns| Some(IndexBuilderState::new(ns.clone())))
}

/// Opens the builder `make` returns for the selected collection (`None` opens nothing). Refused
/// on a read-only connection, and while another modal of the index writes is open or a write is
/// in flight: one at a time, and nothing opens until it is answered.
pub(super) fn open_with(
    state: AppState,
    make: impl FnOnce(&AppState, &Namespace) -> Option<IndexBuilderState>,
) -> Out {
    open_checked(state, make, None)
}

/// [`open_with`], showing `refused` as a banner when `make` returns nothing (the click was for
/// something that is gone). The already-busy and read-only refusals are decided before `make`.
pub(super) fn open_checked(
    state: AppState,
    make: impl FnOnce(&AppState, &Namespace) -> Option<IndexBuilderState>,
    refused: Option<Banner>,
) -> Out {
    gated(state, |s| {
        if s.active_is_read_only() {
            let banner = Some(Banner::ReadOnly);
            return (AppState { banner, ..s }, Vec::new());
        }
        let Some(ns) = s.selected_ns().cloned() else {
            return (s, Vec::new());
        };
        if s.indexes.has_prompt() || s.indexes.writing.is_some() || s.write_warning.is_some() {
            return (s, Vec::new());
        }
        let Some(builder) = make(&s, &ns) else {
            let banner = refused.or_else(|| s.banner.clone());
            return (AppState { banner, ..s }, Vec::new());
        };
        let session = s.session_id();
        let held = s.indexes.ns.as_ref() == Some(&ns);
        let base = match held {
            true => s.indexes,
            false => s.indexes.for_collection(ns.clone()),
        };
        // Opened from Browse or the Queries view, the Indexes tab may never have listed this
        // collection; the builder checks for an existing index only against a loaded list. A
        // real collection always has `_id_`, so an empty list is one not loaded. The request
        // keeps its number: a history read already on its way is still answered.
        let list_wanted = base.indexes.is_empty() && !base.loading;
        let list = list_wanted.then(|| Command::ListIndexes {
            session,
            ns: ns.clone(),
            request: base.request,
        });
        let base = match list_wanted {
            true => base.awaiting_list(),
            false => base,
        };
        let indexes = base.with_builder(Some(builder));
        let count = Command::EstimatedCount { session, ns };
        (
            AppState { indexes, ..s },
            [Some(count), list].into_iter().flatten().collect(),
        )
    })
}

/// Closes the builder and forgets what was typed.
pub(super) fn cancel_builder(state: AppState) -> Out {
    // A write is parked for the production warning: the builder it came from stays as it is.
    if state.write_warning.is_some() {
        return (state, Vec::new());
    }
    let indexes = state.indexes.with_builder(None);
    (AppState { indexes, ..state }, Vec::new())
}

/// Applies one edit to the open builder; with none open the edit is dropped.
pub(super) fn edit_builder(state: AppState, edit: BuilderEdit) -> Out {
    let Some(builder) = state
        .indexes
        .builder
        .clone()
        .filter(|_| state.write_warning.is_none())
    else {
        return (state, Vec::new());
    };
    let indexes = state.indexes.with_builder(Some(builder.edited(edit)));
    (AppState { indexes, ..state }, Vec::new())
}

/// Sends `CreateIndex` for what the builder describes, through the write guard: refused on a
/// read-only connection, parked for the production warning on a production one. Nothing is sent
/// while the builder is invalid, another write is in flight or the collection's index list has not
/// arrived (its duplicate check would run against nothing); the builder stays open either way.
pub(super) fn create(state: AppState) -> Out {
    if state.indexes.list_unloaded() {
        return (state, Vec::new());
    }
    let spec = state
        .indexes
        .builder
        .as_ref()
        .filter(|b| state.selected_ns() == Some(&b.ns))
        .and_then(|b| b.spec_among(&state.indexes.indexes).ok());
    let Some(spec) = spec else {
        return (state, Vec::new());
    };
    // The original of a copy is recorded now, from the builder being sent.
    let replaces = state
        .indexes
        .builder
        .as_ref()
        .and_then(|b| b.replaces().map(str::to_string));
    let intended = Some(IndexWrite::Create {
        name: spec.name.clone(),
        replaces,
    });
    let state = AppState {
        indexes: state.indexes.intending(intended),
        ..state
    };
    mutation(
        state,
        None,
        WriteKind::CreateIndex,
        Guard::Warn,
        |session, ns| Command::CreateIndex { session, ns, spec },
    )
}

/// The document count of `ns` arrived. An answer for another collection, or with no builder open,
/// is dropped.
pub(super) fn estimate_arrived(state: AppState, ns: Namespace, count: u64) -> AppState {
    let Some(builder) = state.indexes.builder.clone().filter(|b| b.ns == ns) else {
        return state;
    };
    let indexes = state
        .indexes
        .with_builder(Some(builder.with_estimate(count)));
    AppState { indexes, ..state }
}

/// `command` was just sent (immediately, or confirmed after the production warning): the write
/// is recorded as what it was, so its answer acts on that and not on whatever is open by then.
pub(super) fn sent(state: AppState, command: &Command) -> AppState {
    // A profiler change is no index write: the Queries view tracks it on its own.
    if let Command::SetProfilingLevel { db, .. } = command {
        return super::queries_actions::write_sent(state, db);
    }
    let (ns, write) = match command {
        Command::CreateIndex { ns, spec, .. } => {
            let write = match state.indexes.intended.clone() {
                Some(IndexWrite::Create { name, replaces }) if name == spec.name => {
                    IndexWrite::Create { name, replaces }
                }
                _ => IndexWrite::Create {
                    name: spec.name.clone(),
                    replaces: None,
                },
            };
            (ns, write)
        }
        Command::DropIndex { ns, name, .. } => {
            let write = match state.indexes.intended.clone() {
                Some(IndexWrite::Drop {
                    name: intended,
                    then_select,
                }) if intended == *name => IndexWrite::Drop {
                    name: intended,
                    then_select,
                },
                _ => IndexWrite::Drop {
                    name: name.clone(),
                    then_select: None,
                },
            };
            (ns, write)
        }
        Command::SetIndexHidden { ns, name, .. } => (ns, IndexWrite::Hide { name: name.clone() }),
        Command::SetIndexTtl { ns, name, .. } => (ns, IndexWrite::Ttl { name: name.clone() }),
        _ => return state,
    };
    let indexes = state.indexes.intending(None).write_sent(ns, write);
    AppState { indexes, ..state }
}

/// The index write in flight failed: its modals can be edited and the write sent again.
pub(super) fn write_failed(state: AppState) -> AppState {
    let indexes = state.indexes.write_answered(false);
    AppState {
        indexes,
        pending: None,
        ..state
    }
}

/// An index write on `ns` succeeded: the write in flight is settled (the lists are asked for
/// again by the caller). A stray answer, with no write of ours pending on `ns`, changes nothing.
pub(super) fn write_finished(state: AppState, ns: &Namespace) -> AppState {
    if !state.pending.as_ref().is_some_and(|p| p.ns == *ns) {
        return state;
    }
    let indexes = state.indexes.write_answered(true);
    let state = AppState {
        indexes,
        pending: None,
        ..state
    };
    // The collection has a new index: the aggregate's hint picker asks for the list again.
    match state.selected_ns() == Some(ns) {
        true => AppState {
            aggregate: Box::new(state.aggregate.hint_edited(HintListUi::reset)),
            ..state
        },
        false => state,
    }
}
