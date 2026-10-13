//! The database-level pairing helper's view: what it offers for the two picked databases and
//! which of those pairs are already in the list. Pure and egui-free.
use std::collections::HashSet;

use curlybrace_core::compare::pairing::{PairingOptions, Suggestion, suggest};
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::store::Namespace;

use super::pairs::PairList;
use super::setup::SetupForm;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DatabasePairing {
    pub include_system: bool,
}

/// One line of the helper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairingRow {
    Matched { name: String, paired: bool },
    NoCounterpart { side: Side, name: String },
}

impl DatabasePairing {
    /// Matched names first, then those without a counterpart; empty until both databases are
    /// picked.
    pub fn rows(&self, setup: &SetupForm, list: &PairList) -> Vec<PairingRow> {
        let paired = paired_names(setup, list);
        self.suggestions(setup)
            .into_iter()
            .map(|s| match s {
                Suggestion::Matched { name } => {
                    let paired = paired.contains(name.as_str());
                    PairingRow::Matched { name, paired }
                }
                Suggestion::OnlyInSource { name } => PairingRow::NoCounterpart {
                    side: Side::Source,
                    name,
                },
                Suggestion::OnlyInTarget { name } => PairingRow::NoCounterpart {
                    side: Side::Target,
                    name,
                },
            })
            .collect()
    }

    /// The matched pairs the list does not hold yet.
    pub fn matched_to_add(
        &self,
        setup: &SetupForm,
        list: &PairList,
    ) -> Vec<(Namespace, Namespace)> {
        let (Some(sdb), Some(tdb)) = (&setup.source.database, &setup.target.database) else {
            return Vec::new();
        };
        self.rows(setup, list)
            .into_iter()
            .filter_map(|row| match row {
                PairingRow::Matched {
                    name,
                    paired: false,
                } => Some((
                    Namespace::new(sdb.clone(), name.clone()),
                    Namespace::new(tdb.clone(), name),
                )),
                _ => None,
            })
            .collect()
    }

    fn suggestions(&self, setup: &SetupForm) -> Vec<Suggestion> {
        if setup.source.database.is_none() || setup.target.database.is_none() {
            return Vec::new();
        }
        let opts = PairingOptions {
            include_system: self.include_system,
        };
        suggest(&setup.source.collections, &setup.target.collections, opts)
    }
}

/// The names already paired same-name across the two picked databases; built once per call.
fn paired_names<'a>(setup: &SetupForm, list: &'a PairList) -> HashSet<&'a str> {
    let (Some(sdb), Some(tdb)) = (&setup.source.database, &setup.target.database) else {
        return HashSet::new();
    };
    list.entries
        .iter()
        .filter(|e| {
            e.source.db == *sdb && e.target.db == *tdb && e.source.collection == e.target.collection
        })
        .map(|e| e.source.collection.as_str())
        .collect()
}

#[cfg(test)]
#[path = "pairing_view_tests.rs"]
mod tests;
