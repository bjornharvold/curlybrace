//! The hint picker's index list in the reducer: one `ListIndexes` of its own (numbered from
//! `HINT_REQUEST_BASE`) when the Options section first opens, or the Indexes tab's list when it
//! already holds this collection. Its answer and failure never reach the Indexes tab: a failure
//! of any kind, a lost server included, is shown inline as "could not list" (the preview's own
//! read notices a lost server).
use super::aggregate_preview_send::target;
use super::reducer::Out;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::aggregate_hint_list::{HintListUi, is_hint_request};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::CollectionIndex;

/// `state` with the active draft's hint list replaced by `f(list)`; the list is moved, not cloned.
fn with_hint(mut state: AppState, f: impl FnOnce(HintListUi) -> HintListUi) -> AppState {
    let ui = &mut state.aggregate.options_ui;
    ui.hint = f(std::mem::take(&mut ui.hint));
    state
}

/// The section wants the list: nothing when it was asked for already; the Indexes tab's names
/// when it holds the selected collection; otherwise one read. Without a connection to ask the
/// list is "could not list".
pub(super) fn wanted(state: AppState) -> Out {
    if !state.aggregate.options_ui.hint.wants_list() {
        return (state, Vec::new());
    }
    let Some((session, ns)) = target(&state) else {
        return (with_hint(state, HintListUi::unavailable), Vec::new());
    };
    let held = state.indexes.ns.as_ref() == Some(&ns)
        && !state.indexes.loading
        && !state.indexes.indexes.is_empty();
    if held {
        let names = names_of(&state.indexes.indexes);
        return (with_hint(state, |h| h.from_names(names)), Vec::new());
    }
    let mut asked = 0;
    let state = with_hint(state, |h| {
        let (h, request) = h.requested();
        asked = request;
        h
    });
    let command = Command::ListIndexes {
        session,
        ns,
        request: asked,
    };
    (state, vec![command])
}

fn names_of(indexes: &[CollectionIndex]) -> Vec<String> {
    indexes.iter().map(|i| i.spec.name.clone()).collect()
}

/// Whether `ev` answers or fails a hint-list read (by its request number alone).
pub(super) fn is_hint_list_event(ev: &Event) -> bool {
    match ev {
        Event::Indexes { request, .. } => is_hint_request(*request),
        Event::RequestFailed {
            context: "list_indexes",
            request,
            ..
        } => is_hint_request(*request),
        _ => false,
    }
}

/// The answer or failure, applied to its session's draft (active or parked).
pub(super) fn on_event(state: AppState, ev: Event) -> Out {
    let Some(session) = ev.session() else {
        return (state, Vec::new());
    };
    state.with_session(session, |s| {
        if s.status != ConnStatus::Connected {
            return (s, Vec::new());
        }
        let next = match ev {
            Event::Indexes {
                ns,
                indexes,
                request,
                ..
            } => match s.selected_ns() == Some(&ns) {
                true => {
                    let names = names_of(&indexes);
                    with_hint(s, |h| h.arrived(request, names))
                }
                false => s,
            },
            Event::RequestFailed { request, .. } => with_hint(s, |h| h.failed(request)),
            _ => s,
        };
        (next, Vec::new())
    })
}

#[cfg(test)]
#[path = "aggregate_hint_actions_tests.rs"]
mod tests;
