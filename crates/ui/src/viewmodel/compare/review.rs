//! The review an apply starts from: a snapshot of what will be written where, the typed
//! confirmation it needs and the warnings it carries. Pure; the caller resolves connection
//! names, so a connection renamed or deleted afterwards cannot change what was confirmed.
use std::collections::BTreeSet;

use curlybrace_core::compare::apply::{ApplyKind, Direction, IdPolicy};
use curlybrace_core::compare::key::KeySpec;
use curlybrace_core::compare::protocol::PairId;
use curlybrace_core::compare::results::Category;
use curlybrace_core::compare::rules::{CompareRules, FieldScope};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

use super::CompareState;
use super::pairs::EntryId;
use super::pending::{PendingChange, PendingList, Scope};
use super::selection::RowAction;

/// Where one write lands, by name (a snapshot).
#[derive(Clone, Debug, PartialEq)]
pub struct Destination {
    pub connection: String,
    pub database: String,
    pub collection: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KindCounts {
    pub insert: u64,
    pub replace: u64,
    pub delete: u64,
}

impl KindCounts {
    pub fn total(&self) -> u64 {
        self.insert + self.replace + self.delete
    }

    fn plus(self, kind: ApplyKind, n: u64) -> Self {
        match kind {
            ApplyKind::Insert => Self {
                insert: self.insert + n,
                ..self
            },
            ApplyKind::Replace => Self {
                replace: self.replace + n,
                ..self
            },
            ApplyKind::Delete => Self {
                delete: self.delete + n,
                ..self
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReviewPair {
    pub pair: PairId,
    pub entry: EntryId,
    /// Each distinct action of the pair with the place it writes.
    pub destinations: Vec<(RowAction, Destination)>,
    pub counts: KindCounts,
}

/// What the person must type before deletes are confirmed.
#[derive(Clone, Debug, PartialEq)]
pub enum RequiredText {
    /// A collection or database name, typed as it is.
    Literal(String),
    /// The localized confirmation word (`compare.apply.confirm_word`).
    Word,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Warning {
    GeneratedIds,
    KeepsDestinationId,
    WholeDocumentOverwrite,
    MaybeDuplicateBusinessKey,
}

impl Warning {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::GeneratedIds => "compare.apply.warning.generated_ids",
            Self::KeepsDestinationId => "compare.apply.warning.keeps_destination_id",
            Self::WholeDocumentOverwrite => "compare.apply.warning.whole_document_overwrite",
            Self::MaybeDuplicateBusinessKey => "compare.apply.warning.maybe_duplicate_business_key",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Review {
    pub pairs: Vec<ReviewPair>,
    pub id_policy: IdPolicy,
    /// What the person typed so far.
    pub confirm_text: String,
    pub required: Option<RequiredText>,
    pub warnings: Vec<Warning>,
    /// What the warnings derive from, kept so a changed id policy recomputes them.
    pub(super) warn_inputs: Vec<WarnInput>,
}

impl Review {
    /// The text that must be typed, `word` being the localized confirmation word.
    pub fn required_text<'a>(&'a self, word: &'a str) -> Option<&'a str> {
        match self.required.as_ref()? {
            RequiredText::Literal(text) => Some(text),
            RequiredText::Word => Some(word),
        }
    }

    /// Confirm may fire: nothing is required, or the trimmed typed text equals it exactly.
    pub fn confirm_enabled(&self, word: &str) -> bool {
        match self.required_text(word) {
            None => true,
            Some(required) => !required.is_empty() && self.confirm_text.trim() == required,
        }
    }

    pub fn with_confirm_text(self, text: &str) -> Self {
        Self {
            confirm_text: text.to_string(),
            ..self
        }
    }

    pub fn with_id_policy(self, policy: IdPolicy) -> Self {
        let warnings = warnings_for(&self.warn_inputs, policy);
        Self {
            id_policy: policy,
            warnings,
            ..self
        }
    }

    /// Writes the review stands for.
    pub fn op_total(&self) -> u64 {
        self.pairs.iter().map(|p| p.counts.total()).sum()
    }
}

/// The two connections of the session, with the names to show for them.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewConnections {
    pub source: (Uuid, String),
    pub target: (Uuid, String),
}

/// What the review needs of a pair entry.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewEntry {
    pub entry: EntryId,
    pub source: Namespace,
    pub target: Namespace,
    pub key: KeySpec,
    pub rules: CompareRules,
}

/// One written place: connection identity plus names.
type Place = (Uuid, String, String);

struct Resolved<'a> {
    change: &'a PendingChange,
    entry: ReviewEntry,
    count: u64,
}

impl Resolved<'_> {
    fn kind(&self) -> ApplyKind {
        self.change.action.kind_for(self.change.category)
    }

    fn direction(&self) -> Direction {
        self.change.action.direction()
    }

    fn namespace(&self) -> &Namespace {
        match self.direction() {
            Direction::SourceToTarget => &self.entry.target,
            Direction::TargetToSource => &self.entry.source,
        }
    }

    fn destination(&self, conns: &ReviewConnections) -> Destination {
        let ns = self.namespace();
        let (_, name) = match self.direction() {
            Direction::SourceToTarget => &conns.target,
            Direction::TargetToSource => &conns.source,
        };
        Destination {
            connection: name.clone(),
            database: ns.db.clone(),
            collection: ns.collection.clone(),
        }
    }

    fn place(&self, conns: &ReviewConnections) -> Place {
        let (id, _) = match self.direction() {
            Direction::SourceToTarget => &conns.target,
            Direction::TargetToSource => &conns.source,
        };
        let ns = self.namespace();
        (*id, ns.db.clone(), ns.collection.clone())
    }

    fn kinds(&self) -> WarnInput {
        WarnInput {
            kind: self.kind(),
            include_id_without_id_key: self.entry.rules.include_id && !self.entry.key.contains_id(),
            narrow_scope: self.entry.rules.field_scope != FieldScope::All,
            business_key: !self.entry.key.contains_id(),
        }
    }
}

/// What a warning needs of one write group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarnInput {
    kind: ApplyKind,
    include_id_without_id_key: bool,
    narrow_scope: bool,
    business_key: bool,
}

fn warnings_for(inputs: &[WarnInput], policy: IdPolicy) -> Vec<Warning> {
    let any = |f: &dyn Fn(&WarnInput) -> bool| inputs.iter().any(f);
    let insert = |w: &WarnInput| w.kind == ApplyKind::Insert;
    let replace = |w: &WarnInput| w.kind == ApplyKind::Replace;
    let found: BTreeSet<Warning> = [
        (
            policy == IdPolicy::Generate && any(&insert),
            Warning::GeneratedIds,
        ),
        (
            any(&|w| replace(w) && w.include_id_without_id_key),
            Warning::KeepsDestinationId,
        ),
        (
            any(&|w| replace(w) && w.narrow_scope),
            Warning::WholeDocumentOverwrite,
        ),
        (
            any(&|w| insert(w) && w.business_key),
            Warning::MaybeDuplicateBusinessKey,
        ),
    ]
    .into_iter()
    .filter_map(|(on, warning)| on.then_some(warning))
    .collect();
    found.into_iter().collect()
}

/// The text the deletes need typed (ruling 5): one collection names it, several collections of
/// one database name the database, anything wider needs the localized word.
fn required_for(places: &[Place]) -> Option<RequiredText> {
    let distinct: BTreeSet<&Place> = places.iter().collect();
    let first = distinct.iter().next()?;
    if distinct.len() == 1 {
        return Some(RequiredText::Literal(first.2.clone()));
    }
    let one_database = distinct.iter().all(|p| p.0 == first.0 && p.1 == first.1);
    Some(match one_database {
        true => RequiredText::Literal(first.1.clone()),
        false => RequiredText::Word,
    })
}

fn resolve<'a>(
    pending: &'a PendingList,
    entry_of: &dyn Fn(PairId) -> Option<ReviewEntry>,
    totals: &dyn Fn(PairId, Category) -> usize,
) -> Option<Vec<Resolved<'a>>> {
    pending
        .items()
        .iter()
        .map(|change| {
            let count = match &change.scope {
                Scope::Keys(keys) => keys.len() as u64,
                Scope::Category => totals(change.pair, change.category) as u64,
            };
            let entry = entry_of(change.pair)?;
            Some(Resolved {
                change,
                entry,
                count,
            })
        })
        .collect()
}

fn review_pair(group: &[&Resolved], conns: &ReviewConnections) -> ReviewPair {
    let mut destinations: Vec<(RowAction, Destination)> = Vec::new();
    for r in group {
        let item = (r.change.action, r.destination(conns));
        if !destinations.contains(&item) {
            destinations.push(item);
        }
    }
    ReviewPair {
        pair: group[0].change.pair,
        entry: group[0].entry.entry,
        destinations,
        counts: group
            .iter()
            .fold(KindCounts::default(), |c, r| c.plus(r.kind(), r.count)),
    }
}

/// The review of `pending`: `None` when it is empty or names a pair `entry_of` does not know.
/// `totals` gives the size of a category-wide item.
pub fn build_review(
    pending: &PendingList,
    id_policy: IdPolicy,
    conns: &ReviewConnections,
    entry_of: &dyn Fn(PairId) -> Option<ReviewEntry>,
    totals: &dyn Fn(PairId, Category) -> usize,
) -> Option<Review> {
    let resolved = resolve(pending, entry_of, totals)?;
    if resolved.is_empty() {
        return None;
    }
    let mut order: Vec<PairId> = Vec::new();
    for r in &resolved {
        if !order.contains(&r.change.pair) {
            order.push(r.change.pair);
        }
    }
    let pairs = order
        .iter()
        .map(|pair| {
            let group: Vec<&Resolved> =
                resolved.iter().filter(|r| r.change.pair == *pair).collect();
            review_pair(&group, conns)
        })
        .collect();
    let deletes: Vec<Place> = resolved
        .iter()
        .filter(|r| r.kind() == ApplyKind::Delete)
        .map(|r| r.place(conns))
        .collect();
    let warn_inputs: Vec<WarnInput> = resolved.iter().map(Resolved::kinds).collect();
    Some(Review {
        pairs,
        id_policy,
        confirm_text: String::new(),
        required: required_for(&deletes),
        warnings: warnings_for(&warn_inputs, id_policy),
        warn_inputs,
    })
}

impl CompareState {
    /// The review of the pending list, or `None` when review is not allowed or a pending pair
    /// can no longer be resolved. `name_of` resolves a connection id to its display name; it is
    /// asked once, here, and the review keeps the answers.
    pub fn review(&self, id_policy: IdPolicy, name_of: &dyn Fn(Uuid) -> String) -> Option<Review> {
        if !self.can_review() {
            return None;
        }
        let (source, target) = self.connections?;
        let conns = ReviewConnections {
            source: (source, name_of(source)),
            target: (target, name_of(target)),
        };
        let entry_of = |pair: PairId| self.review_entry(pair);
        let totals =
            |pair: PairId, cat: Category| self.results.get(&pair).map_or(0, |v| v.counts.of(cat));
        build_review(&self.pending, id_policy, &conns, &entry_of, &totals)
    }

    fn review_entry(&self, pair: PairId) -> Option<ReviewEntry> {
        let row = self.overview.rows.iter().find(|r| r.pair == pair)?;
        let entry = self.pairs.get(row.entry)?;
        let form = entry.effective(&self.setup);
        Some(ReviewEntry {
            entry: entry.id,
            source: entry.source.clone(),
            target: entry.target.clone(),
            key: form.key().ok()?,
            rules: form.rules().ok()?,
        })
    }
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
