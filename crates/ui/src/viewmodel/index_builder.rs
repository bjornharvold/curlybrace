//! View-model of the index builder: the keys and options being edited, the problems that keep
//! Create disabled and the exact `createIndexes` command the store will receive. Pure and
//! egui-free; the updaters return new values.
use bson::{Bson, Document};
use curlybrace_core::ejson::{doc_to_view_json, editor_json_to_doc};
use curlybrace_core::indexes::{
    IndexKey, IndexOptions, IndexProblem, IndexSpec, KeyKind, validate_new,
};
use curlybrace_core::store::Namespace;

/// A collection with more documents than this gets the "building may take a while" warning.
pub const BUILD_WARNING_THRESHOLD: u64 = 1_000_000;

/// The key types the builder offers. Hashed and 2d keys are shown and preserved when they come
/// from an existing index, but cannot be added here.
pub const BUILDER_KINDS: [KeyKind; 5] = [
    KeyKind::Asc,
    KeyKind::Desc,
    KeyKind::Text,
    KeyKind::Geo2dSphere,
    KeyKind::Wildcard,
];

/// The fields of a text index that only make sense while it has a text key.
const TEXT_FIELDS: [&str; 4] = [
    "weights",
    "default_language",
    "language_override",
    "textIndexVersion",
];

/// The field a wildcard key covers when none is typed: every field.
const WILDCARD_ALL: &str = "$**";

/// A note shown above Create.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildWarning {
    /// Building on this many documents can take a long time and load the server.
    LargeCollection { documents: u64 },
}

/// Why Create is disabled: a problem core found, or one only the builder can see.
#[derive(Clone, Debug, PartialEq)]
pub enum BuilderProblem {
    Index(IndexProblem),
    EmptyName,
    /// The partial filter is not a JSON object.
    PartialFilterInvalid,
    /// The collation is not a JSON object.
    CollationInvalid,
    /// The copy holds a text key whose fields are unknown (the server listed no `weights`), so
    /// the builder cannot rebuild the same index.
    KeyNotRebuildable,
}

impl BuilderProblem {
    /// The `en.json` key of the message; `DuplicateField` also takes the field as `{field}`.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Index(IndexProblem::NoKeys) => "indexes.builder.problem_no_keys",
            Self::Index(IndexProblem::EmptyField) => "indexes.builder.problem_empty_field",
            Self::Index(IndexProblem::DuplicateField(_)) => "indexes.builder.problem_duplicate",
            Self::Index(IndexProblem::TtlNeedsOneOrderedKey) => "indexes.builder.problem_ttl_keys",
            Self::Index(IndexProblem::TtlNotPositive) => "indexes.builder.problem_ttl_positive",
            Self::EmptyName => "indexes.builder.problem_empty_name",
            Self::PartialFilterInvalid => "indexes.builder.problem_partial_filter",
            Self::CollationInvalid => "indexes.builder.problem_collation",
            Self::KeyNotRebuildable => "indexes.builder.problem_unrebuildable",
        }
    }

    /// The offending field of a duplicate-field problem.
    pub fn field(&self) -> Option<&str> {
        match self {
            Self::Index(IndexProblem::DuplicateField(field)) => Some(field),
            _ => None,
        }
    }
}

/// One change to the builder, as the view reports it.
#[derive(Clone, Debug, PartialEq)]
pub enum BuilderEdit {
    AddKey,
    RemoveKey(usize),
    /// Moves the key at `from` so it ends up at position `to`.
    MoveKey {
        from: usize,
        to: usize,
    },
    SetKind(usize, KeyKind),
    SetField(usize, String),
    SetName(String),
    SetUnique(bool),
    SetSparse(bool),
    SetHidden(bool),
    SetTtl(Option<i64>),
    SetPartialFilterText(String),
    SetCollationText(String),
}

/// The index being built on `ns`.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexBuilderState {
    pub ns: Namespace,
    pub keys: Vec<IndexKey>,
    /// Flags and TTL. The partial filter and collation live in their text boxes below until
    /// `spec` parses them.
    pub options: IndexOptions,
    pub partial_filter_text: String,
    pub collation_text: String,
    pub name: String,
    /// The person typed a name; until then the name follows the keys.
    pub name_edited: bool,
    pub warning: Option<BuildWarning>,
    /// The collection's estimated document count, once the server has answered.
    pub estimated_documents: Option<u64>,
    /// Create was sent for what this builder holds and the server has not answered: the fields
    /// are frozen so what is shown stays what was sent.
    pub submitted: bool,
    /// The index this builder makes a copy of. Its `raw` rides along into the copy, so fields the
    /// builder does not model (weights, bits, min, max, ...) survive.
    pub editing: Option<IndexSpec>,
    /// The name a copy was offered with, to tell whether anything was changed.
    pub suggested_name: String,
}

impl IndexBuilderState {
    /// A builder with one empty ascending key row.
    pub fn new(ns: Namespace) -> Self {
        let keys = vec![IndexKey {
            field: String::new(),
            kind: KeyKind::Asc,
        }];
        Self {
            ns,
            name: default_name(&keys),
            keys,
            options: IndexOptions::default(),
            partial_filter_text: String::new(),
            collation_text: String::new(),
            name_edited: false,
            warning: None,
            estimated_documents: None,
            submitted: false,
            editing: None,
            suggested_name: String::new(),
        }
    }

    /// A builder pre-filled from `original`, to be created under `name`.
    pub fn from_copy(ns: Namespace, original: &IndexSpec, name: String) -> Self {
        let text = |filter: &Option<Document>| filter.as_ref().map(doc_to_view_json);
        Self {
            keys: original.keys.clone(),
            options: original.options.clone(),
            partial_filter_text: text(&original.options.partial_filter).unwrap_or_default(),
            collation_text: text(&original.options.collation).unwrap_or_default(),
            suggested_name: name.clone(),
            name,
            // The copy keeps its name whatever is done to the keys.
            name_edited: true,
            editing: Some(original.clone()),
            ..Self::new(ns)
        }
    }

    /// The name of the index this builder is a copy of.
    pub fn replaces(&self) -> Option<&str> {
        self.editing.as_ref().map(|s| s.name.as_str())
    }

    /// The same builder marked as sent (or, with `false`, editable again).
    pub fn with_submitted(self, submitted: bool) -> Self {
        Self { submitted, ..self }
    }

    /// Applies one edit; a submitted builder takes none.
    pub fn edited(self, edit: BuilderEdit) -> Self {
        if self.submitted {
            return self;
        }
        match edit {
            BuilderEdit::AddKey => self.adding_key(),
            BuilderEdit::RemoveKey(i) => self.removing_key(i),
            BuilderEdit::MoveKey { from, to } => self.moving_key(from, to),
            BuilderEdit::SetKind(i, kind) => self.with_kind(i, kind),
            BuilderEdit::SetField(i, text) => self.with_field(i, text),
            BuilderEdit::SetName(name) => self.with_name(name),
            BuilderEdit::SetUnique(on) => self.with_unique(on),
            BuilderEdit::SetSparse(on) => self.with_sparse(on),
            BuilderEdit::SetHidden(on) => self.with_hidden(on),
            BuilderEdit::SetTtl(seconds) => self.with_ttl(seconds),
            BuilderEdit::SetPartialFilterText(text) => self.with_partial_filter_text(text),
            BuilderEdit::SetCollationText(text) => self.with_collation_text(text),
        }
    }

    pub fn adding_key(self) -> Self {
        let key = IndexKey {
            field: String::new(),
            kind: KeyKind::Asc,
        };
        let keys = [self.keys.clone(), vec![key]].concat();
        self.with_keys(keys)
    }

    pub fn removing_key(self, index: usize) -> Self {
        if index >= self.keys.len() {
            return self;
        }
        let mut keys = self.keys.clone();
        keys.remove(index);
        self.with_keys(keys)
    }

    pub fn moving_key(self, from: usize, to: usize) -> Self {
        if from >= self.keys.len() || to >= self.keys.len() || from == to {
            return self;
        }
        let mut keys = self.keys.clone();
        let key = keys.remove(from);
        keys.insert(to, key);
        self.with_keys(keys)
    }

    /// A key of a copy that the picker cannot offer (hashed, 2d) keeps its type.
    pub fn with_kind(self, index: usize, kind: KeyKind) -> Self {
        if self.is_fixed(index) {
            return self;
        }
        self.with_key_at(index, |key| IndexKey { kind, ..key })
    }

    /// Whether the key at `index` is one the builder only shows and preserves.
    pub fn is_fixed(&self, index: usize) -> bool {
        self.keys.get(index).is_some_and(|key| {
            matches!(
                key.kind,
                KeyKind::Hashed | KeyKind::Geo2d | KeyKind::Other(_)
            )
        })
    }

    /// The field text is stored as typed; trimming happens when the spec is made.
    pub fn with_field(self, index: usize, text: impl Into<String>) -> Self {
        if self.is_fixed(index) {
            return self;
        }
        let field = text.into();
        self.with_key_at(index, |key| IndexKey { field, ..key })
    }

    /// A typed name sticks; clearing the box hands the name back to the keys.
    pub fn with_name(self, name: impl Into<String>) -> Self {
        let name = name.into();
        match name.trim().is_empty() {
            true => Self {
                name: default_name(&self.keys),
                name_edited: false,
                ..self
            },
            false => Self {
                name,
                name_edited: true,
                ..self
            },
        }
    }

    pub fn with_unique(self, unique: bool) -> Self {
        self.with_options(|options| IndexOptions { unique, ..options })
    }

    pub fn with_sparse(self, sparse: bool) -> Self {
        self.with_options(|options| IndexOptions { sparse, ..options })
    }

    pub fn with_hidden(self, hidden: bool) -> Self {
        self.with_options(|options| IndexOptions { hidden, ..options })
    }

    pub fn with_ttl(self, seconds: Option<i64>) -> Self {
        self.with_options(|options| IndexOptions {
            expire_after_seconds: seconds,
            ..options
        })
    }

    pub fn with_partial_filter_text(self, text: impl Into<String>) -> Self {
        Self {
            partial_filter_text: text.into(),
            ..self
        }
    }

    pub fn with_collation_text(self, text: impl Into<String>) -> Self {
        Self {
            collation_text: text.into(),
            ..self
        }
    }

    /// The server's estimated document count arrived.
    pub fn with_estimate(self, documents: u64) -> Self {
        let warning = (documents > BUILD_WARNING_THRESHOLD)
            .then_some(BuildWarning::LargeCollection { documents });
        Self {
            estimated_documents: Some(documents),
            warning,
            ..self
        }
    }

    /// The spec to create, or what is wrong with the builder. The first problem found wins: keys,
    /// then name, then the filter and collation texts.
    pub fn spec(&self) -> Result<IndexSpec, BuilderProblem> {
        if self.has_unreadable_text_key() {
            return Err(BuilderProblem::KeyNotRebuildable);
        }
        let partial_filter = self.filter(&self.partial_filter_text, |o| &o.partial_filter);
        let collation = self.filter(&self.collation_text, |o| &o.collation);
        let spec = self.draft(
            partial_filter.clone().unwrap_or_default(),
            collation.clone().unwrap_or_default(),
        );
        validate_new(&spec).map_err(BuilderProblem::Index)?;
        if spec.name.is_empty() {
            return Err(BuilderProblem::EmptyName);
        }
        if partial_filter.is_none() {
            return Err(BuilderProblem::PartialFilterInvalid);
        }
        if collation.is_none() {
            return Err(BuilderProblem::CollationInvalid);
        }
        Ok(spec)
    }

    /// A text key listed without its fields (`Other("text")` on the `_fts` marker): rebuilding it
    /// would create a text index on a field literally named `_fts`, a different index.
    fn has_unreadable_text_key(&self) -> bool {
        self.keys
            .iter()
            .any(|key| matches!(&key.kind, KeyKind::Other(kind) if kind == "text"))
    }

    /// What keeps Create disabled, if anything.
    pub fn problem(&self) -> Option<BuilderProblem> {
        self.spec().err()
    }

    /// The command Create would send, as pretty extended JSON. It is rendered from the very
    /// `create_command` the store receives; a text that does not parse is left out of it, since
    /// Create is disabled then anyway.
    pub fn preview_json(&self) -> String {
        let spec = self.draft(
            self.filter(&self.partial_filter_text, |o| &o.partial_filter)
                .flatten(),
            self.filter(&self.collation_text, |o| &o.collation)
                .flatten(),
        );
        doc_to_view_json(&spec.create_command(&self.ns.collection))
    }

    /// Nothing was typed or changed yet: closing the modal by accident loses nothing.
    pub fn is_untouched(&self) -> bool {
        let fresh = match &self.editing {
            Some(original) => {
                Self::from_copy(self.ns.clone(), original, self.suggested_name.clone())
            }
            None => Self::new(self.ns.clone()),
        };
        Self {
            warning: None,
            estimated_documents: None,
            ..self.clone()
        } == fresh
    }

    /// The spec without any validation, filter and collation given.
    fn draft(&self, partial_filter: Option<Document>, collation: Option<Document>) -> IndexSpec {
        IndexSpec {
            name: self.name.trim().to_string(),
            keys: self.keys.iter().map(effective_key).collect(),
            options: IndexOptions {
                partial_filter,
                collation,
                ..self.options.clone()
            },
            raw: self.plugin_fields(),
        }
    }

    /// The original's unmodelled fields, made to agree with the current keys: `weights` lists
    /// exactly the text keys (original weights kept, 1 for a new one) and goes with the other text
    /// options when no text key is left; `2dsphereIndexVersion` goes with the last 2dsphere key.
    /// Hashed and 2d keys cannot be edited, so their fields (`bits`, `min`, `max`) stay.
    fn plugin_fields(&self) -> Document {
        let Some(original) = &self.editing else {
            return Document::new();
        };
        let mut raw = original.raw.clone();
        let has = |kind: KeyKind| self.keys.iter().any(|k| k.kind == kind);
        if !has(KeyKind::Text) {
            for field in TEXT_FIELDS {
                raw.remove(field);
            }
        } else if let Ok(old) = original.raw.get_document("weights") {
            let weights: Document = self
                .keys
                .iter()
                .filter(|k| k.kind == KeyKind::Text)
                .map(|k| {
                    let field = k.field.trim().to_string();
                    let weight = old.get(&field).cloned().unwrap_or(Bson::Int32(1));
                    (field, weight)
                })
                .collect();
            raw.insert("weights", weights);
        }
        if !has(KeyKind::Geo2dSphere) {
            raw.remove("2dsphereIndexVersion");
        }
        raw
    }

    /// A filter or collation text as a document: blank is none, text that is not a JSON object is
    /// `None`. Text that still reads as the original's document gives the original itself back,
    /// so a copy repeats the definition exactly (number types included) until it is edited.
    fn filter(
        &self,
        text: &str,
        of: impl Fn(&IndexOptions) -> &Option<Document>,
    ) -> Option<Option<Document>> {
        let original = self.editing.as_ref().and_then(|o| of(&o.options).as_ref());
        match original {
            Some(document) if text == doc_to_view_json(document) => Some(Some(document.clone())),
            _ => parse_object(text),
        }
    }

    /// The keys replaced; the name follows them until the person has typed one.
    fn with_keys(self, keys: Vec<IndexKey>) -> Self {
        let name = match self.name_edited {
            true => self.name.clone(),
            false => default_name(&keys),
        };
        Self { keys, name, ..self }
    }

    fn with_key_at(self, index: usize, change: impl FnOnce(IndexKey) -> IndexKey) -> Self {
        if index >= self.keys.len() {
            return self;
        }
        let mut keys = self.keys.clone();
        keys[index] = change(keys[index].clone());
        self.with_keys(keys)
    }

    fn with_options(self, change: impl FnOnce(IndexOptions) -> IndexOptions) -> Self {
        let options = change(self.options.clone());
        Self { options, ..self }
    }
}

/// `None` for a text that is not a JSON object; `Some(None)` for blank text.
fn parse_object(text: &str) -> Option<Option<Document>> {
    match text.trim().is_empty() {
        true => Some(None),
        false => editor_json_to_doc(text).ok().map(Some),
    }
}

/// The key as the server will receive it: leading and trailing whitespace of the field dropped,
/// everything inside kept (dots and unicode included), and a wildcard key always ending in `$**`.
fn effective_key(key: &IndexKey) -> IndexKey {
    let typed = key.field.trim();
    let field = match key.kind {
        KeyKind::Wildcard if typed.is_empty() => WILDCARD_ALL.to_string(),
        KeyKind::Wildcard if !typed.ends_with(WILDCARD_ALL) => format!("{typed}.{WILDCARD_ALL}"),
        _ => typed.to_string(),
    };
    IndexKey {
        field,
        kind: key.kind.clone(),
    }
}

/// MongoDB's default index name: each `field_direction` joined by `_`.
pub fn default_name(keys: &[IndexKey]) -> String {
    keys.iter()
        .map(effective_key)
        .map(|key| format!("{}_{}", key.field, name_suffix(&key.kind)))
        .collect::<Vec<_>>()
        .join("_")
}

fn name_suffix(kind: &KeyKind) -> &str {
    match kind {
        KeyKind::Asc | KeyKind::Wildcard => "1",
        KeyKind::Desc => "-1",
        KeyKind::Text => "text",
        KeyKind::Geo2dSphere => "2dsphere",
        KeyKind::Hashed => "hashed",
        KeyKind::Geo2d => "2d",
        KeyKind::Other(kind) => kind,
    }
}

/// The `en.json` key of a key type's label.
pub fn kind_label_key(kind: &KeyKind) -> &'static str {
    match kind {
        KeyKind::Asc => "indexes.builder.kind_asc",
        KeyKind::Desc => "indexes.builder.kind_desc",
        KeyKind::Text => "indexes.builder.kind_text",
        KeyKind::Geo2dSphere => "indexes.builder.kind_geo_sphere",
        KeyKind::Wildcard => "indexes.builder.kind_wildcard",
        KeyKind::Hashed => "indexes.builder.kind_hashed",
        KeyKind::Geo2d => "indexes.builder.kind_geo_flat",
        KeyKind::Other(_) => "indexes.builder.kind_other",
    }
}

#[cfg(test)]
#[path = "index_builder_tests.rs"]
mod tests;
