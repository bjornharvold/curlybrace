//! Reducer arms of the index writes besides Create: editing a copy, hiding, changing the TTL
//! and dropping. All of them go through the write guard like every other write (refused on a
//! read-only connection, parked for the production warning), none touches `_id_`, and what an
//! answer does was decided when the write was SENT (see `index_write_actions::sent`).
use super::actions::{gated, mutation};
use super::index_write_actions::open_with;
use super::reducer::Out;
use super::state::{AppState, Banner};
use super::write_guard::{Guard, WriteKind};
use crate::viewmodel::index_write::{copy_name, is_id_index, ttl_problem};
use crate::viewmodel::{DropConfirm, IndexBuilderState, TtlPrompt};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::CollectionIndex;

/// The listed index `name`, unless it is `_id_`, which no write may touch.
fn writable<'a>(state: &'a AppState, name: &str) -> Option<&'a CollectionIndex> {
    if is_id_index(name) || state.indexes.ns.as_ref() != state.selected_ns() {
        return None;
    }
    state.indexes.indexes.iter().find(|i| i.spec.name == name)
}

/// Nothing else is open or in flight, so a new prompt may open.
fn idle(state: &AppState) -> bool {
    !state.indexes.has_prompt() && state.indexes.writing.is_none()
}

/// A write is in flight for one of the modals: they hold still until it is answered.
fn busy(state: &AppState) -> bool {
    state.indexes.writing.is_some()
}

fn refuse_read_only(state: AppState) -> Out {
    let banner = Some(Banner::ReadOnly);
    (AppState { banner, ..state }, Vec::new())
}

fn unchanged(state: AppState) -> Out {
    (state, Vec::new())
}

/// Opens the builder pre-filled with a copy of `name`, which keeps serving queries until the
/// copy is built and the person agrees to drop it.
pub(super) fn edit_copy(state: AppState, name: String) -> Out {
    open_with(state, |s, ns| {
        let original = writable(s, &name)?.spec.clone();
        let copy = copy_name(&s.indexes.indexes, &original.name);
        Some(IndexBuilderState::from_copy(ns.clone(), &original, copy))
    })
}

/// Drops the index a copy replaced, through the guard. The name is the one recorded when the
/// copy was built, not the selection.
pub(super) fn confirm_drop_old(state: AppState) -> Out {
    let Some(old) = state.indexes.drop_old.as_ref().map(|d| d.old.clone()) else {
        return unchanged(state);
    };
    send_drop(state, old)
}

/// Keeps both indexes: the question is closed and nothing is sent.
pub(super) fn keep_old(state: AppState) -> Out {
    if busy(&state) {
        return unchanged(state);
    }
    let indexes = state.indexes.with_drop_old(None);
    (AppState { indexes, ..state }, Vec::new())
}

pub(super) fn set_hidden(state: AppState, name: String, hidden: bool) -> Out {
    if writable(&state, &name).is_none() {
        return unchanged(state);
    }
    mutation(
        state,
        None,
        WriteKind::HideIndex,
        Guard::Warn,
        |session, ns| Command::SetIndexHidden {
            session,
            ns,
            name,
            hidden,
        },
    )
}

/// Opens the expiry input of a TTL index, showing its current value.
pub(super) fn change_ttl(state: AppState, name: String) -> Out {
    gated(state, |s| {
        if s.active_is_read_only() {
            return refuse_read_only(s);
        }
        let current = writable(&s, &name).and_then(|i| i.spec.options.expire_after_seconds);
        let (true, Some(_)) = (idle(&s), current) else {
            return unchanged(s);
        };
        let indexes = s
            .indexes
            .with_ttl_prompt(Some(TtlPrompt::new(name, current)));
        (AppState { indexes, ..s }, Vec::new())
    })
}

pub(super) fn ttl_typed(state: AppState, text: String) -> Out {
    let Some(prompt) = state.indexes.ttl_prompt.clone().filter(|_| !busy(&state)) else {
        return unchanged(state);
    };
    let indexes = state.indexes.with_ttl_prompt(Some(prompt.with_text(text)));
    (AppState { indexes, ..state }, Vec::new())
}

pub(super) fn ttl_cancelled(state: AppState) -> Out {
    if busy(&state) {
        return unchanged(state);
    }
    let indexes = state.indexes.with_ttl_prompt(None);
    (AppState { indexes, ..state }, Vec::new())
}

/// Changes the expiry in place (`collMod`), not a copy. Only an index that already expires has
/// one to change, and the seconds must be positive (the builder's rule and wording).
pub(super) fn set_ttl(state: AppState, name: String, seconds: i64) -> Out {
    let expires =
        writable(&state, &name).is_some_and(|i| i.spec.options.expire_after_seconds.is_some());
    if !expires || ttl_problem(seconds).is_some() {
        return unchanged(state);
    }
    mutation(
        state,
        None,
        WriteKind::SetIndexTtl,
        Guard::Warn,
        |session, ns| Command::SetIndexTtl {
            session,
            ns,
            name,
            seconds,
        },
    )
}

/// Opens the typed confirmation of dropping `name`.
pub(super) fn drop_requested(state: AppState, name: String) -> Out {
    gated(state, |s| {
        if s.active_is_read_only() {
            return refuse_read_only(s);
        }
        if writable(&s, &name).is_none() || !idle(&s) {
            return unchanged(s);
        }
        let indexes = s.indexes.with_drop_confirm(Some(DropConfirm::new(name)));
        (AppState { indexes, ..s }, Vec::new())
    })
}

pub(super) fn drop_typed(state: AppState, text: String) -> Out {
    let Some(confirm) = state.indexes.drop_confirm.clone().filter(|_| !busy(&state)) else {
        return unchanged(state);
    };
    let indexes = state
        .indexes
        .with_drop_confirm(Some(confirm.with_typed(text)));
    (AppState { indexes, ..state }, Vec::new())
}

/// Drops the index of the open confirmation, once its exact name was typed.
pub(super) fn drop_confirmed(state: AppState) -> Out {
    let name = state
        .indexes
        .drop_confirm
        .as_ref()
        .filter(|c| c.confirmed())
        .map(|c| c.name.clone());
    match name {
        Some(name) => send_drop(state, name),
        None => unchanged(state),
    }
}

pub(super) fn drop_cancelled(state: AppState) -> Out {
    if busy(&state) {
        return unchanged(state);
    }
    let indexes = state.indexes.with_drop_confirm(None);
    (AppState { indexes, ..state }, Vec::new())
}

/// `DropIndex` for `name` through the guard; never for `_id_`.
fn send_drop(state: AppState, name: String) -> Out {
    if is_id_index(&name) {
        return unchanged(state);
    }
    mutation(
        state,
        None,
        WriteKind::DropIndex,
        Guard::Warn,
        |session, ns| Command::DropIndex { session, ns, name },
    )
}
