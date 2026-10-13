//! What the pair list and the defaults make of every entry, worked out once: the views memoise
//! it (it parses keys and filters, which is not free for hundreds of entries) and the gates
//! read it. A pure function of the list, the defaults and whether both sides share a connection.
use super::pairs::{EntryId, PairList};
use super::setup::{SetupError, SetupForm};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PairsDigest {
    /// The first issue of each entry, in list order.
    pub issues: Vec<Option<SetupError>>,
    /// The included entries whose spec builds: what a check queues.
    pub valid: Vec<EntryId>,
}

impl PairsDigest {
    pub fn of(list: &PairList, defaults: &SetupForm, same_connection: bool) -> Self {
        let issues: Vec<Option<SetupError>> = list
            .entries
            .iter()
            .map(|e| e.issues(defaults, same_connection).into_iter().next())
            .collect();
        let valid = list
            .entries
            .iter()
            .zip(&issues)
            .filter(|(e, issue)| e.included && issue.is_none())
            .map(|(e, _)| e.id)
            .collect();
        Self { issues, valid }
    }

    /// The issue of the entry at `index` of the list.
    pub fn issue_at(&self, index: usize) -> Option<SetupError> {
        self.issues.get(index).copied().flatten()
    }
}

#[cfg(test)]
#[path = "digest_tests.rs"]
mod tests;
