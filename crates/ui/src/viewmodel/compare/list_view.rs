//! The pair list as the setup and preflight screens need it, worked out once per change of its
//! inputs: each entry's issue, the included rows, and what the whole-database helper offers.
//! The views memoise it by [`ListInputs`] so a frame costs O(visible rows). Pure.
use curlybrace_core::compare::protocol::Side;

use super::CompareState;
use super::digest::PairsDigest;
use super::pairing_view::{DatabasePairing, PairingRow};
use super::pairs::PairList;
use super::setup::SetupForm;

/// A collection that exists on one side only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unmatched {
    pub side: Side,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListView {
    pub digest: PairsDigest,
    /// Indices (into the list) of the included entries.
    pub included: Vec<usize>,
    /// Names without a counterpart in the two picked databases.
    pub unmatched: Vec<Unmatched>,
    /// How many same-named pairs Add all would add.
    pub addable: usize,
}

impl ListView {
    pub fn of(state: &CompareState) -> Self {
        let rows = state.pairing.rows(&state.setup, &state.pairs);
        let addable = rows
            .iter()
            .filter(|r| matches!(r, PairingRow::Matched { paired: false, .. }))
            .count();
        let unmatched = rows
            .into_iter()
            .filter_map(|row| match row {
                PairingRow::NoCounterpart { side, name } => Some(Unmatched { side, name }),
                PairingRow::Matched { .. } => None,
            })
            .collect();
        let included = (0..state.pairs.entries.len())
            .filter(|i| state.pairs.entries[*i].included)
            .collect();
        Self {
            digest: state.digest(),
            included,
            unmatched,
            addable,
        }
    }

    pub fn can_add_all(&self) -> bool {
        self.addable > 0
    }
}

/// What a [`ListView`] is a function of; kept next to it to tell whether it is still current.
#[derive(Clone, Debug, PartialEq)]
pub struct ListInputs {
    pairs: PairList,
    setup: SetupForm,
    pairing: DatabasePairing,
    one_connection: bool,
}

impl ListInputs {
    pub fn of(state: &CompareState) -> Self {
        Self {
            pairs: state.pairs.clone(),
            setup: state.setup.clone(),
            pairing: state.pairing.clone(),
            one_connection: state.one_connection(),
        }
    }

    /// Whether a view built from these inputs is still the view of `state`.
    pub fn is_for(&self, state: &CompareState) -> bool {
        self.pairs == state.pairs
            && self.setup == state.setup
            && self.pairing == state.pairing
            && self.one_connection == state.one_connection()
    }
}

/// Pair-with-this-name needs a name to pair with.
pub fn can_pair_with(typed: &str) -> bool {
    !typed.trim().is_empty()
}

impl CompareState {
    /// Add pair may fire: both pickers hold a collection (no views) and the list lacks the pair.
    pub fn can_add_pair(&self) -> bool {
        self.setup
            .pick_pair()
            .is_ok_and(|(source, target)| !self.pairs.has_pair(&source, &target))
    }
}

#[cfg(test)]
#[path = "list_view_tests.rs"]
mod tests;
