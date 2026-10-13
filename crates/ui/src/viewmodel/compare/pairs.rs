//! The list of collection pairs the Compare workspace will run, each following the shared
//! defaults unless the user overrode a field for that pair. Pure; every edit returns a new list.
use curlybrace_core::compare::protocol::{PairId, PairSpec, Side};
use curlybrace_core::store::{CollectionKind, Namespace};

use super::setup::{RulesForm, SetupError, SetupForm, SideForm};

/// Identifies an entry in the list; never reused within a list.
pub type EntryId = u32;

/// What one pair does differently from the defaults. `None` follows the defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PairOverrides {
    pub key_text: Option<String>,
    pub source_filter: Option<String>,
    pub target_filter: Option<String>,
    pub rules: Option<RulesForm>,
}

impl PairOverrides {
    pub fn with_key_text(self, text: Option<String>) -> Self {
        Self {
            key_text: text,
            ..self
        }
    }

    pub fn with_filter(self, side: Side, text: Option<String>) -> Self {
        match side {
            Side::Source => Self {
                source_filter: text,
                ..self
            },
            Side::Target => Self {
                target_filter: text,
                ..self
            },
        }
    }

    pub fn with_rules(self, rules: Option<RulesForm>) -> Self {
        Self { rules, ..self }
    }

    pub fn is_customised(&self) -> bool {
        self.key_text.is_some()
            || self.source_filter.is_some()
            || self.target_filter.is_some()
            || self.rules.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PairEntry {
    pub id: EntryId,
    pub source: Namespace,
    pub target: Namespace,
    pub included: bool,
    pub overrides: PairOverrides,
}

impl PairEntry {
    /// The defaults with this entry's namespaces and overrides applied, as a form that can
    /// parse itself.
    pub fn effective(&self, defaults: &SetupForm) -> SetupForm {
        let o = &self.overrides;
        SetupForm {
            source: side_for(defaults.side(Side::Source), &self.source, &o.source_filter),
            target: side_for(defaults.side(Side::Target), &self.target, &o.target_filter),
            key_text: o
                .key_text
                .clone()
                .unwrap_or_else(|| defaults.key_text.clone()),
            rules: o.rules.clone().unwrap_or_else(|| defaults.rules.clone()),
        }
    }

    /// What is wrong with this entry. Its views are found in the defaults' listing without
    /// copying it, so checking hundreds of entries stays cheap.
    pub fn issues(&self, defaults: &SetupForm, same_connection: bool) -> Vec<SetupError> {
        let mut issues: Vec<SetupError> = [Side::Source, Side::Target]
            .into_iter()
            .filter(|side| self.is_view_on(*side, defaults))
            .map(SetupError::ViewSelected)
            .collect();
        let effective = self.effective_without_listing(defaults);
        issues.extend(effective.issues());
        if issues.is_empty() && same_connection && effective.same_namespace_and_filter() {
            issues.push(SetupError::SameCollection);
        }
        issues
    }

    fn namespace(&self, side: Side) -> &Namespace {
        match side {
            Side::Source => &self.source,
            Side::Target => &self.target,
        }
    }

    /// The side's namespace is a view in the defaults' listing (known only for the database
    /// the defaults' picker holds).
    fn is_view_on(&self, side: Side, defaults: &SetupForm) -> bool {
        let ns = self.namespace(side);
        let listing = defaults.side(side);
        listing.database.as_deref() == Some(ns.db.as_str())
            && listing
                .collections
                .iter()
                .any(|(n, kind)| *n == ns.collection && *kind == CollectionKind::View)
    }

    /// `effective` over defaults stripped of their listings: enough to parse key, filters and
    /// rules without copying the (possibly long) lists of databases and collections.
    fn effective_without_listing(&self, defaults: &SetupForm) -> SetupForm {
        let bare = |side: Side| {
            let full = defaults.side(side);
            SideForm {
                database: full.database.clone(),
                filter_text: full.filter_text.clone(),
                ..SideForm::default()
            }
        };
        let stripped = SetupForm {
            source: bare(Side::Source),
            target: bare(Side::Target),
            key_text: defaults.key_text.clone(),
            rules: defaults.rules.clone(),
        };
        self.effective(&stripped)
    }

    pub fn spec(
        &self,
        id: PairId,
        defaults: &SetupForm,
        same_connection: bool,
    ) -> Result<PairSpec, SetupError> {
        match self.issues(defaults, same_connection).into_iter().next() {
            Some(first) => Err(first),
            None => self.effective(defaults).pair(id),
        }
    }
}

/// One side of the effective form: the defaults' side pointed at `ns`. The collection listing
/// is only kept when it belongs to that database.
fn side_for(default: &SideForm, ns: &Namespace, filter: &Option<String>) -> SideForm {
    let same_db = default.database.as_deref() == Some(ns.db.as_str());
    SideForm {
        database: Some(ns.db.clone()),
        collection: Some(ns.collection.clone()),
        collections: if same_db {
            default.collections.clone()
        } else {
            Vec::new()
        },
        filter_text: filter
            .clone()
            .unwrap_or_else(|| default.filter_text.clone()),
        ..default.clone()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PairList {
    pub entries: Vec<PairEntry>,
    pub next_entry: EntryId,
    pub editing: Option<EntryId>,
}

impl PairList {
    /// Appends an included pair following the defaults; an exact duplicate is ignored.
    pub fn added(self, source: Namespace, target: Namespace) -> Self {
        if self
            .entries
            .iter()
            .any(|e| e.source == source && e.target == target)
        {
            return self;
        }
        let entry = PairEntry {
            id: self.next_entry,
            source,
            target,
            included: true,
            overrides: PairOverrides::default(),
        };
        Self {
            next_entry: self.next_entry + 1,
            entries: self.entries.into_iter().chain([entry]).collect(),
            ..self
        }
    }

    pub fn removed(self, id: EntryId) -> Self {
        Self {
            editing: self.editing.filter(|e| *e != id),
            entries: self.entries.into_iter().filter(|e| e.id != id).collect(),
            ..self
        }
    }

    pub fn included(self, id: EntryId, on: bool) -> Self {
        self.map_entries(|e| match e.id == id {
            true => PairEntry { included: on, ..e },
            false => e,
        })
    }

    /// Re-run: only this entry stays ticked.
    pub fn only_included(self, id: EntryId) -> Self {
        self.map_entries(|e| PairEntry {
            included: e.id == id,
            ..e
        })
    }

    pub fn with_overrides(self, id: EntryId, overrides: PairOverrides) -> Self {
        let mut pending = Some(overrides);
        self.map_entries(|e| {
            if e.id != id {
                return e;
            }
            match pending.take() {
                Some(overrides) => PairEntry { overrides, ..e },
                None => e,
            }
        })
    }

    pub fn editing(self, id: Option<EntryId>) -> Self {
        Self {
            editing: id,
            ..self
        }
    }

    /// The exact pair is already in the list.
    pub fn has_pair(&self, source: &Namespace, target: &Namespace) -> bool {
        self.entries
            .iter()
            .any(|e| e.source == *source && e.target == *target)
    }

    pub fn get(&self, id: EntryId) -> Option<&PairEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn map_entries(self, f: impl FnMut(PairEntry) -> PairEntry) -> Self {
        Self {
            entries: self.entries.into_iter().map(f).collect(),
            ..self
        }
    }
}

#[cfg(test)]
#[path = "pairs_tests.rs"]
mod tests;
