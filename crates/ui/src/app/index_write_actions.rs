//! Reducer arms of the index builder: opening it, editing it and creating the index through the
//! same write guard as every other write, and what the runtime answers to that.
use super::actions::{gated, mutation};
use super::reducer::Out;
use super::state::{AppState, Banner};
use super::write_guard::{Guard, WriteKind};
use crate::viewmodel::{BuilderEdit, IndexBuilderState};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;

/// Opens the builder on the selected collection and asks for its document count, which only
/// decides whether the large-collection warning shows. A read-only connection never gets one.
pub(super) fn open_builder(state: AppState) -> Out {
    gated(state, |s| {
        if s.active_is_read_only() {
            let banner = Some(Banner::ReadOnly);
            return (AppState { banner, ..s }, Vec::new());
        }
        let Some(ns) = s.selected_ns().cloned() else {
            return (s, Vec::new());
        };
        if s.indexes.builder.is_some() {
            return (s, Vec::new());
        }
        let session = s.session_id();
        let held = s.indexes.ns.as_ref() == Some(&ns);
        let base = match held {
            true => s.indexes,
            false => s.indexes.for_collection(ns.clone()),
        };
        let indexes = base.with_builder(Some(IndexBuilderState::new(ns.clone())));
        let request = Command::EstimatedCount { session, ns };
        (AppState { indexes, ..s }, vec![request])
    })
}

/// Closes the builder and forgets what was typed.
pub(super) fn cancel_builder(state: AppState) -> Out {
    let indexes = state.indexes.with_builder(None);
    (AppState { indexes, ..state }, Vec::new())
}

/// Applies one edit to the open builder; with none open the edit is dropped.
pub(super) fn edit_builder(state: AppState, edit: BuilderEdit) -> Out {
    let Some(builder) = state.indexes.builder.clone() else {
        return (state, Vec::new());
    };
    let indexes = state.indexes.with_builder(Some(builder.edited(edit)));
    (AppState { indexes, ..state }, Vec::new())
}

/// Sends `CreateIndex` for what the builder describes, through the write guard: refused on a
/// read-only connection, parked for the production warning on a production one. Nothing is sent
/// while the builder is invalid or another write is in flight; the builder stays open either way.
pub(super) fn create(state: AppState) -> Out {
    let spec = state
        .indexes
        .builder
        .as_ref()
        .filter(|b| state.selected_ns() == Some(&b.ns))
        .and_then(|b| b.spec().ok());
    let Some(spec) = spec else {
        return (state, Vec::new());
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

/// An index write on `ns` succeeded. When it was the create in flight, the builder closes and the
/// new index is the selected one (the lists are asked for again by the caller).
pub(super) fn create_finished(state: AppState, ns: &Namespace) -> AppState {
    if !state.pending.as_ref().is_some_and(|p| p.ns == *ns) {
        return state;
    }
    let created = state
        .indexes
        .builder
        .as_ref()
        .filter(|b| b.ns == *ns)
        .and_then(|b| b.spec().ok())
        .map(|spec| spec.name);
    let indexes = match created {
        Some(name) => state.indexes.with_builder(None).selecting(name),
        None => state.indexes,
    };
    AppState {
        indexes,
        pending: None,
        ..state
    }
}
