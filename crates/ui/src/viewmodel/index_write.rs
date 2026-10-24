//! View-model of the index writes besides Create: what is in flight, the confirmations that
//! come before a drop or a TTL change, and the names of copies. Pure and egui-free; the
//! updaters return new values.
use super::index_builder::BuilderProblem;
use curlybrace_core::indexes::IndexProblem;
use curlybrace_core::store::CollectionIndex;

/// The index every collection has; the server refuses to drop, hide or change it.
pub const ID_INDEX: &str = "_id_";

/// Whether `name` is the `_id_` index, which none of the write controls may touch.
pub fn is_id_index(name: &str) -> bool {
    name == ID_INDEX
}

/// The one index write that was sent and has not been answered. It is recorded at send time (also
/// after the production confirm) and is what the answer acts on, whatever is open by then.
#[derive(Clone, Debug, PartialEq)]
pub enum IndexWrite {
    /// `name` is being built. `replaces` is the index it is a copy of, so a success asks about
    /// dropping that one.
    Create {
        name: String,
        replaces: Option<String>,
    },
    /// `name` is being dropped; a success selects `then_select` (the copy that replaced it).
    Drop {
        name: String,
        then_select: Option<String>,
    },
    Hide {
        name: String,
    },
    Ttl {
        name: String,
    },
}

/// The typed confirmation of a drop: the index goes only when its exact name was typed.
#[derive(Clone, Debug, PartialEq)]
pub struct DropConfirm {
    pub name: String,
    pub typed: String,
}

impl DropConfirm {
    pub fn new(name: String) -> Self {
        Self {
            name,
            typed: String::new(),
        }
    }

    pub fn with_typed(self, typed: String) -> Self {
        Self { typed, ..self }
    }

    /// The typed text is the index name, character for character.
    pub fn confirmed(&self) -> bool {
        self.typed == self.name
    }
}

/// The question after a copy was built: drop the index it replaces?
#[derive(Clone, Debug, PartialEq)]
pub struct DropOld {
    pub old: String,
    pub new: String,
}

/// The small number input of "Change TTL...".
#[derive(Clone, Debug, PartialEq)]
pub struct TtlPrompt {
    pub name: String,
    pub text: String,
}

impl TtlPrompt {
    /// A prompt for `name`, showing its current expiry.
    pub fn new(name: String, current: Option<i64>) -> Self {
        let text = current.map(|s| s.to_string()).unwrap_or_default();
        Self { name, text }
    }

    pub fn with_text(self, text: String) -> Self {
        Self { text, ..self }
    }

    /// The seconds typed, or the problem that keeps Apply disabled.
    pub fn seconds(&self) -> Result<i64, BuilderProblem> {
        let seconds = self
            .text
            .trim()
            .parse::<i64>()
            .map_err(|_| negative_ttl())?;
        ttl_problem(seconds).map_or(Ok(seconds), Err)
    }
}

/// Whether `seconds` is a valid expiry; the reducer re-checks what the view sends.
pub fn ttl_problem(seconds: i64) -> Option<BuilderProblem> {
    (seconds < 0).then(negative_ttl)
}

/// The problem the builder reports for the same input, so both say the same words.
fn negative_ttl() -> BuilderProblem {
    BuilderProblem::Index(IndexProblem::TtlNegative)
}

/// The name of a copy of `base` that no index in `existing` has: `base_v2`, `base_v3`, ... A
/// `base` that already ends in `_vN` continues its sequence after N (`a_v2` becomes `a_v3`) rather
/// than stacking suffixes. A version that is taken is skipped, never reused.
pub fn copy_name(existing: &[CollectionIndex], base: &str) -> String {
    let (stem, version) = split_version(base);
    let first = version.map_or(2, |n| n.saturating_add(1).max(2));
    (first..=u64::MAX)
        .map(|n| format!("{stem}_v{n}"))
        .find(|name| existing.iter().all(|i| i.spec.name != *name))
        .unwrap_or_else(|| format!("{stem}_v{first}_copy"))
}

/// `a_v3` is `("a", Some(3))`; anything without a numeric `_vN` tail after a non-empty stem is
/// all stem.
fn split_version(name: &str) -> (&str, Option<u64>) {
    let parsed = name
        .rsplit_once("_v")
        .filter(|(stem, digits)| !stem.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|(stem, digits)| Some((stem, digits.parse().ok()?)));
    match parsed {
        Some((stem, n)) => (stem, Some(n)),
        None => (name, None),
    }
}

#[cfg(test)]
#[path = "index_write_tests.rs"]
mod tests;
