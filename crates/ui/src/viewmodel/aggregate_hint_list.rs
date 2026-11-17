//! The index names the hint picker lists: the collection's indexes, read once when the Options
//! section first opens (or taken from the Indexes tab when it holds the same collection). A
//! numbered read of its own, so it never touches the Indexes tab's state. Every op returns a new
//! value; index names are user data and are never logged.
use std::sync::Arc;

/// Hint-list requests are numbered from here: the Indexes tab counts up from 0 and never gets
/// near it, so an answer is routed by its number alone and the two never settle each other's
/// requests.
pub const HINT_REQUEST_BASE: u64 = 1 << 62;

/// Whether `request` is one of the hint list's.
pub fn is_hint_request(request: u64) -> bool {
    request >= HINT_REQUEST_BASE
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum HintList {
    #[default]
    NotAsked,
    Loading {
        request: u64,
    },
    Loaded(Arc<[String]>),
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintListUi {
    pub list: HintList,
    /// The offset of the next request number; it only grows, so a number is never reused.
    next: u64,
}

impl HintListUi {
    /// Nothing was asked for yet.
    pub fn wants_list(&self) -> bool {
        self.list == HintList::NotAsked
    }

    /// The list is being read under a new request number.
    pub fn requested(self) -> (Self, u64) {
        let request = HINT_REQUEST_BASE + self.next;
        (
            Self {
                list: HintList::Loading { request },
                next: self.next + 1,
            },
            request,
        )
    }

    /// The names are known without asking the server (the Indexes tab holds them).
    pub fn from_names(self, names: Vec<String>) -> Self {
        Self {
            list: HintList::Loaded(names.into()),
            ..self
        }
    }

    /// The answer to `request`; any other changes nothing.
    pub fn arrived(self, request: u64, names: Vec<String>) -> Self {
        match self.list {
            HintList::Loading { request: awaited } if awaited == request => self.from_names(names),
            _ => self,
        }
    }

    /// The awaited `request` failed; any other changes nothing.
    pub fn failed(self, request: u64) -> Self {
        match self.list {
            HintList::Loading { request: awaited } if awaited == request => self.unavailable(),
            _ => self,
        }
    }

    /// The list cannot be read (no connection to ask).
    pub fn unavailable(self) -> Self {
        Self {
            list: HintList::Failed,
            ..self
        }
    }

    /// A failed list may be asked for again.
    pub fn retried(self) -> Self {
        match self.list {
            HintList::Failed => self.reset(),
            _ => self,
        }
    }

    /// Forgets the list (another collection or connection); the counter stays.
    pub fn reset(self) -> Self {
        Self {
            list: HintList::NotAsked,
            ..self
        }
    }

    /// The names listed, none until loaded.
    pub fn names(&self) -> &[String] {
        match &self.list {
            HintList::Loaded(names) => names,
            _ => &[],
        }
    }
}

#[cfg(test)]
#[path = "aggregate_hint_list_tests.rs"]
mod tests;
