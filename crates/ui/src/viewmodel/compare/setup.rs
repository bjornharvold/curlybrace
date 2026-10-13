//! The compare setup form: what the user picked and typed, and what it parses to. Pure; every
//! problem is a [`SetupError`] that names an i18n key, never English text.
use bson::Document;
use curlybrace_core::compare::key::KeySpec;
use curlybrace_core::compare::protocol::{PairId, PairSpec, Side};
use curlybrace_core::compare::rules::{CompareRules, FieldScope};
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::store::{CollectionKind, Namespace};

/// Why the form cannot produce a [`PairSpec`] yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// A side has no database or collection picked.
    Incomplete,
    /// A side picked a view, which cannot be compared.
    ViewSelected(Side),
    Key,
    Filter(Side),
    Rules,
    /// Both sides are the same collection on the same connection with the same filter.
    SameCollection,
}

impl SetupError {
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Incomplete => "compare.setup.incomplete",
            Self::ViewSelected(_) => "compare.setup.view_selected",
            Self::Key => "compare.setup.key_invalid",
            Self::Filter(_) => "compare.setup.filter_invalid",
            Self::Rules => "compare.setup.rules_invalid",
            Self::SameCollection => "compare.setup.same_collection",
        }
    }
}

/// What the user clicked in a side's pickers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    Database(String),
    Collection(String),
}

/// One side of the compare: the pickers, the lists they offer and the filter text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SideForm {
    pub databases: Vec<String>,
    pub database: Option<String>,
    pub collections: Vec<(String, CollectionKind)>,
    pub collection: Option<String>,
    pub filter_text: String,
}

impl SideForm {
    pub fn with_databases(self, names: Vec<String>) -> Self {
        Self {
            databases: names,
            ..self
        }
    }

    /// Picking a database drops the collection and its listing (they belonged to the old one).
    pub fn with_database(self, name: &str) -> Self {
        if self.database.as_deref() == Some(name) {
            return self;
        }
        Self {
            database: Some(name.to_string()),
            collections: Vec::new(),
            collection: None,
            ..self
        }
    }

    /// Accepts the listing only for the database currently picked.
    pub fn with_collections(self, db: &str, entries: Vec<(String, CollectionKind)>) -> Self {
        if self.database.as_deref() != Some(db) {
            return self;
        }
        Self {
            collections: entries,
            ..self
        }
    }

    pub fn with_collection(self, name: &str) -> Self {
        Self {
            collection: Some(name.to_string()),
            ..self
        }
    }

    pub fn with_filter(self, text: &str) -> Self {
        Self {
            filter_text: text.to_string(),
            ..self
        }
    }

    pub fn picked(&self) -> Option<Namespace> {
        match (&self.database, &self.collection) {
            (Some(db), Some(c)) => Some(Namespace::new(db.clone(), c.clone())),
            _ => None,
        }
    }

    /// The picked collection is a view (unknown kinds count as plain collections).
    pub fn is_view(&self) -> bool {
        let Some(name) = &self.collection else {
            return false;
        };
        self.collections
            .iter()
            .any(|(n, kind)| n == name && *kind == CollectionKind::View)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeKind {
    #[default]
    All,
    AllExcept,
    Only,
}

/// The rules the user can change. Defaults equal `CompareRules::for_key`.
#[derive(Clone, Debug, PartialEq)]
pub struct RulesForm {
    pub scope: ScopeKind,
    /// Comma-separated dotted paths for `AllExcept` / `Only`.
    pub paths_text: String,
    pub ignore_array_order: bool,
    pub numbers_equal_across_types: bool,
    pub ignore_field_order: bool,
    /// `None` follows the key (on when the key contains `_id`); `Some` is the user's choice.
    pub include_id: Option<bool>,
}

impl Default for RulesForm {
    fn default() -> Self {
        let d = CompareRules::for_key(&KeySpec::id());
        Self {
            scope: ScopeKind::All,
            paths_text: String::new(),
            ignore_array_order: d.ignore_array_order,
            numbers_equal_across_types: d.numbers_equal_across_types,
            ignore_field_order: d.ignore_field_order,
            include_id: None,
        }
    }
}

impl RulesForm {
    /// The rules for `key`, validated.
    pub fn to_rules(&self, key: &KeySpec) -> Result<CompareRules, SetupError> {
        let paths = split_list(&self.paths_text);
        let field_scope = match self.scope {
            ScopeKind::All => FieldScope::All,
            ScopeKind::AllExcept => FieldScope::AllExcept(paths),
            ScopeKind::Only => FieldScope::Only(paths),
        };
        let rules = CompareRules {
            field_scope,
            ignore_array_order: self.ignore_array_order,
            numbers_equal_across_types: self.numbers_equal_across_types,
            ignore_field_order: self.ignore_field_order,
            include_id: self.include_id.unwrap_or_else(|| key.contains_id()),
        };
        rules.validate().map_err(|_| SetupError::Rules)?;
        Ok(rules)
    }
}

/// `"a, b.c"` to `["a", "b.c"]`; empty text is no entries. Entries stay unvalidated.
fn split_list(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split(',').map(|p| p.trim().to_string()).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetupForm {
    pub source: SideForm,
    pub target: SideForm,
    pub key_text: String,
    pub rules: RulesForm,
}

impl Default for SetupForm {
    fn default() -> Self {
        Self {
            source: SideForm::default(),
            target: SideForm::default(),
            key_text: "_id".to_string(),
            rules: RulesForm::default(),
        }
    }
}

impl SetupForm {
    pub fn side(&self, side: Side) -> &SideForm {
        match side {
            Side::Source => &self.source,
            Side::Target => &self.target,
        }
    }

    pub fn with_side(self, side: Side, form: SideForm) -> Self {
        match side {
            Side::Source => Self {
                source: form,
                ..self
            },
            Side::Target => Self {
                target: form,
                ..self
            },
        }
    }

    pub fn with_key_text(self, text: &str) -> Self {
        Self {
            key_text: text.to_string(),
            ..self
        }
    }

    pub fn with_rules(self, rules: RulesForm) -> Self {
        Self { rules, ..self }
    }

    pub fn key(&self) -> Result<KeySpec, SetupError> {
        let fields = split_list(&self.key_text);
        KeySpec::new(fields).map_err(|_| SetupError::Key)
    }

    /// Empty text means "no filter"; otherwise a JSON object.
    pub fn filter(&self, side: Side) -> Result<Document, SetupError> {
        let text = self.side(side).filter_text.trim();
        if text.is_empty() {
            return Ok(Document::new());
        }
        editor_json_to_doc(text).map_err(|_| SetupError::Filter(side))
    }

    pub fn rules(&self) -> Result<CompareRules, SetupError> {
        self.rules.to_rules(&self.key()?)
    }

    /// Both sides pick the same namespace with equal (parsed) filters. Whether they are also
    /// on the same connection is for the caller to know.
    pub fn same_namespace_and_filter(&self) -> bool {
        let picked = self.source.picked();
        picked.is_some()
            && picked == self.target.picked()
            && matches!(
                (self.filter(Side::Source), self.filter(Side::Target)),
                (Ok(s), Ok(t)) if s == t
            )
    }

    /// Everything wrong with the form, in display order; empty when it is ready.
    pub fn issues(&self) -> Vec<SetupError> {
        let mut issues = Vec::new();
        if self.source.picked().is_none() || self.target.picked().is_none() {
            issues.push(SetupError::Incomplete);
        }
        for side in [Side::Source, Side::Target] {
            if self.side(side).is_view() {
                issues.push(SetupError::ViewSelected(side));
            }
        }
        issues.extend(self.default_issues());
        issues
    }

    /// What is wrong with the shared fields alone (key, filters, rules), ignoring the pickers:
    /// the pickers only feed Add pair, so the defaults panel reports just these.
    pub fn default_issues(&self) -> Vec<SetupError> {
        let mut issues = Vec::new();
        issues.extend(self.key().err());
        for side in [Side::Source, Side::Target] {
            issues.extend(self.filter(side).err());
        }
        issues.extend(self.rules().err().filter(|e| !issues.contains(e)));
        issues
    }

    pub fn ready(&self) -> bool {
        self.issues().is_empty()
    }

    /// The namespaces to add as a pair: both sides picked and neither a view.
    pub fn pick_pair(&self) -> Result<(Namespace, Namespace), SetupError> {
        for side in [Side::Source, Side::Target] {
            if self.side(side).is_view() {
                return Err(SetupError::ViewSelected(side));
            }
        }
        match (self.source.picked(), self.target.picked()) {
            (Some(source), Some(target)) => Ok((source, target)),
            _ => Err(SetupError::Incomplete),
        }
    }

    /// The pair to compare under `id`, or the first thing that is wrong with the form itself.
    pub fn pair(&self, id: PairId) -> Result<PairSpec, SetupError> {
        if let Some(first) = self.issues().into_iter().next() {
            return Err(first);
        }
        let key = self.key()?;
        let rules = self.rules.to_rules(&key)?;
        let (Some(source), Some(target)) = (self.source.picked(), self.target.picked()) else {
            return Err(SetupError::Incomplete);
        };
        Ok(PairSpec {
            id,
            source,
            target,
            source_filter: self.filter(Side::Source)?,
            target_filter: self.filter(Side::Target)?,
            key,
            rules,
        })
    }
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
