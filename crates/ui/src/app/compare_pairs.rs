//! Pair-list actions of the Compare reducer. The pickers and the defaults on the setup form are
//! the source of truth: the pickers only feed Add pair, and an entry never changes because a
//! picker did. Any change to the list or the defaults makes earlier verdicts stale.
use super::compare::{cmd, none, ready, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{
    CompareState, EntryId, PairList, PairOverrides, SetupForm, SideForm,
};
use curlybrace_core::compare::protocol::{CompareCommand, PairId, Side};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::{CollectionKind, Namespace};

/// Replaces the pair list; a change that alters nothing is dropped, any other invalidates.
fn list_edit(state: AppState, change: impl FnOnce(PairList) -> PairList) -> Out {
    let c = &state.compare;
    let next = change(c.pairs.clone());
    if next == c.pairs {
        return none(state);
    }
    let compare = CompareState {
        pairs: next,
        ..c.clone()
    }
    .invalidated();
    none(with_compare(state, compare))
}

/// Replaces the defaults (key, rules, filters); an unchanged form is dropped, a changed one
/// invalidates the verdicts. Entries keep their overrides.
pub(super) fn defaults_edit(state: AppState, change: impl FnOnce(SetupForm) -> SetupForm) -> Out {
    let c = &state.compare;
    let setup = change(c.setup.clone());
    if setup == c.setup {
        return none(state);
    }
    let compare = CompareState { setup, ..c.clone() }.invalidated();
    none(with_compare(state, compare))
}

/// Adds the pair the pickers describe; unpicked sides or a view change nothing (the setup
/// issues already say why).
pub(super) fn add_pair(state: AppState) -> Out {
    match state.compare.setup.pick_pair() {
        Ok((source, target)) => list_edit(state, |list| list.added(source, target)),
        Err(_) => none(state),
    }
}

/// Adds every same-named pair the helper offers and the list lacks, in one transition.
pub(super) fn add_all_same_named(state: AppState) -> Out {
    let c = &state.compare;
    let wanted = c.pairing.matched_to_add(&c.setup, &c.pairs);
    list_edit(state, |list| {
        wanted
            .into_iter()
            .fold(list, |list, (source, target)| list.added(source, target))
    })
}

/// A collection with no counterpart on `side`, paired with `as_name` in the other side's
/// picked database (a missing collection counts as empty). Views are never paired.
pub(super) fn add_unmatched(state: AppState, side: Side, name: &str, as_name: &str) -> Out {
    let as_name = as_name.trim();
    let setup = &state.compare.setup;
    let other = match side {
        Side::Source => Side::Target,
        Side::Target => Side::Source,
    };
    let (own, counterpart) = (
        namespace(setup.side(side), name),
        namespace(setup.side(other), as_name),
    );
    let (Some(own), Some(counterpart)) = (own, counterpart) else {
        return none(state);
    };
    if as_name.is_empty() || is_view(setup.side(side), name) || is_view(setup.side(other), as_name)
    {
        return none(state);
    }
    let (source, target) = match side {
        Side::Source => (own, counterpart),
        Side::Target => (counterpart, own),
    };
    list_edit(state, |list| list.added(source, target))
}

fn namespace(side: &SideForm, name: &str) -> Option<Namespace> {
    let db = side.database.clone()?;
    Some(Namespace::new(db, name.to_string()))
}

fn is_view(side: &SideForm, name: &str) -> bool {
    side.collections
        .iter()
        .any(|(n, kind)| n == name && *kind == CollectionKind::View)
}

/// Removes an entry unless a run or check is active or its pair is queued or running; its results are discarded with it.
pub(super) fn remove_pair(state: AppState, entry: EntryId) -> Out {
    let c = &state.compare;
    if !c.can_remove(entry) {
        return none(state);
    }
    let (overview, held) = c.overview.clone().removed_entry(entry);
    let held: Vec<PairId> = held.into_iter().collect();
    let commands = discards(ready(&state), &held);
    let compare = CompareState {
        overview,
        pairs: c.pairs.clone().removed(entry),
        ..c.clone()
    }
    .without_results(&held)
    .invalidated();
    // Removing one entry makes only that entry's duplicates panel obsolete.
    let compare = CompareState {
        duplicates: state.compare.duplicates.superseded_for(entry),
        ..compare
    };
    (with_compare(state, compare), commands)
}

/// A `Discard` per pair, once the session is ready.
pub(super) fn discards(session: Option<u64>, pairs: &[PairId]) -> Vec<Command> {
    let Some(session) = session else {
        return Vec::new();
    };
    pairs
        .iter()
        .map(|pair| {
            cmd(CompareCommand::Discard {
                session,
                pair: *pair,
            })
        })
        .collect()
}

pub(super) fn include_pair(state: AppState, entry: EntryId, on: bool) -> Out {
    list_edit(state, |list| list.included(entry, on))
}

pub(super) fn set_overrides(state: AppState, entry: EntryId, overrides: PairOverrides) -> Out {
    list_edit(state, |list| list.with_overrides(entry, overrides))
}

/// Opens (or closes) the editor of one entry; an unknown entry closes it. Not an edit.
pub(super) fn edit_pair(state: AppState, entry: Option<EntryId>) -> Out {
    let c = &state.compare;
    let known = entry.filter(|id| c.pairs.get(*id).is_some());
    let compare = CompareState {
        pairs: c.pairs.clone().editing(known),
        ..c.clone()
    };
    none(with_compare(state, compare))
}

/// `system.*` collections in the pairing helper; the list itself is untouched.
pub(super) fn toggle_system(state: AppState, on: bool) -> Out {
    let c = &state.compare;
    let compare = CompareState {
        pairing: crate::viewmodel::compare::DatabasePairing { include_system: on },
        ..c.clone()
    };
    none(with_compare(state, compare))
}
