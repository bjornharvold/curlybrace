//! View-model of the Indexes tab: which tab of a collection is shown, and what the tab holds.
//! Pure and egui-free; the updaters return new values.
use super::index_builder::{BuilderEdit, IndexBuilderState};
use super::index_detail::{series_matching, series_of};
use super::index_write::{DropConfirm, DropOld, IndexWrite, TtlPrompt};
use curlybrace_core::history::ChartRange;
use curlybrace_core::indexes::IndexSpec;
use curlybrace_core::runtime::{IndexSeries, SamplerStatus};
use curlybrace_core::store::{CollectionIndex, Namespace};
use std::collections::BTreeMap;

/// Which tab of the selected collection the workspace shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollectionTab {
    #[default]
    Documents,
    Indexes,
}

impl CollectionTab {
    /// The `en.json` key of the tab's label.
    pub fn label_key(self) -> &'static str {
        match self {
            Self::Documents => "browser.tab_documents",
            Self::Indexes => "indexes.tab",
        }
    }
}

/// What the Indexes tab holds for the collection it was opened on.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexesState {
    /// The collection the lists below belong to; answers for another one are stale.
    pub ns: Option<Namespace>,
    pub indexes: Vec<CollectionIndex>,
    /// The definition hash of each listed index by name, taken once when the list arrives (a
    /// hash is a SHA-256 over the definition, too much to redo per index per frame). An index
    /// that is missing here, a list set without [`Self::with_indexes`], is hashed on demand.
    pub definition_hashes: BTreeMap<String, String>,
    pub series: Vec<IndexSeries>,
    /// Name of the index whose chart and details are shown.
    pub selected: Option<String>,
    pub range: ChartRange,
    /// The index list has been requested and has not arrived yet.
    pub loading: bool,
    /// The session's sampler status, once it has reported one.
    pub sampler: Option<SamplerStatus>,
    /// The index builder, while its modal is open.
    pub builder: Option<IndexBuilderState>,
    /// The index write that was sent and not yet answered. It is what the answer acts on (the
    /// name to select, the index to ask about dropping), whatever is open by then.
    pub writing: Option<IndexWrite>,
    /// What the create or drop being sent will be recorded as. Set when the write is built (so
    /// before a production park) and taken by `write_sent`; the send never re-reads the modals.
    pub intended: Option<IndexWrite>,
    /// The typed confirmation of a drop, while its modal is open.
    pub drop_confirm: Option<DropConfirm>,
    /// "Drop the old index?", asked after a copy was built.
    pub drop_old: Option<DropOld>,
    /// The number input of "Change TTL...", while it is open.
    pub ttl_prompt: Option<TtlPrompt>,
}

impl Default for IndexesState {
    fn default() -> Self {
        Self {
            ns: None,
            indexes: Vec::new(),
            definition_hashes: BTreeMap::new(),
            series: Vec::new(),
            selected: None,
            range: ChartRange::Day,
            loading: false,
            sampler: None,
            builder: None,
            writing: None,
            intended: None,
            drop_confirm: None,
            drop_old: None,
            ttl_prompt: None,
        }
    }
}

impl IndexesState {
    /// A fresh state for `ns`. The range is the person's choice and the sampler status belongs to
    /// the session, not the collection, so both survive from `self`.
    pub fn for_collection(&self, ns: Namespace) -> Self {
        Self {
            ns: Some(ns),
            range: self.range,
            sampler: self.sampler,
            ..Self::default()
        }
    }

    /// The same state waiting for the lists to arrive.
    pub fn requested(self) -> Self {
        Self {
            loading: true,
            ..self
        }
    }

    /// The listed indexes; the selection is dropped when its index is gone.
    pub fn with_indexes(self, indexes: Vec<CollectionIndex>) -> Self {
        let selected = self
            .selected
            .filter(|name| indexes.iter().any(|i| i.spec.name == *name));
        let definition_hashes = indexes
            .iter()
            .map(|i| (i.spec.name.clone(), i.spec.definition_hash()))
            .collect();
        Self {
            indexes,
            definition_hashes,
            selected,
            loading: false,
            ..self
        }
    }

    /// The series drawn for `spec` (see [`series_of`]), matched on the hash taken when the list
    /// arrived.
    pub fn series_of(&self, spec: &IndexSpec) -> Option<&IndexSeries> {
        match self.definition_hashes.get(&spec.name) {
            Some(hash) => series_matching(&self.series, &spec.name, hash),
            None => series_of(&self.series, spec),
        }
    }

    pub fn with_series(self, series: Vec<IndexSeries>) -> Self {
        Self { series, ..self }
    }

    pub fn selecting(self, name: String) -> Self {
        Self {
            selected: Some(name),
            ..self
        }
    }

    /// A new range. The series on hand were read for the old one, so they are dropped.
    pub fn with_range(self, range: ChartRange) -> Self {
        Self {
            range,
            series: Vec::new(),
            ..self
        }
    }

    pub fn with_sampler(self, status: SamplerStatus) -> Self {
        Self {
            sampler: Some(status),
            ..self
        }
    }

    /// The request failed: nothing is on its way any more.
    pub fn settled(self) -> Self {
        Self {
            loading: false,
            ..self
        }
    }

    /// `write` was sent on `ns`. A create freezes the builder open on `ns` (the one submitted).
    pub fn write_sent(self, ns: &Namespace, write: IndexWrite) -> Self {
        let builder = match (&write, self.builder) {
            (IndexWrite::Create { .. }, Some(b)) if b.ns == *ns => Some(b.with_submitted(true)),
            (_, other) => other,
        };
        Self {
            builder,
            writing: Some(write),
            ..self
        }
    }

    /// The write in flight was answered. What happens is decided by the write that was SENT:
    ///  - a create closes only the builder that was submitted and selects its index (a failure
    ///    thaws that builder); a successful copy also asks about dropping the original;
    ///  - a drop closes its modals and selects the copy that replaced it; a failed drop leaves
    ///    them open, so the person can retry or cancel;
    ///  - hide and TTL have nothing else to settle (a TTL prompt closes on success).
    pub fn write_answered(self, succeeded: bool) -> Self {
        let Some(write) = self.writing.clone() else {
            return self;
        };
        let next = Self {
            writing: None,
            ..self
        };
        match (write, succeeded) {
            (IndexWrite::Create { name, replaces }, ok) => next.create_settled(name, replaces, ok),
            (IndexWrite::Drop { then_select, .. }, true) => {
                let next = Self {
                    drop_confirm: None,
                    drop_old: None,
                    ..next
                };
                match then_select {
                    Some(name) => next.selecting(name),
                    None => next,
                }
            }
            (IndexWrite::Ttl { .. }, true) => Self {
                ttl_prompt: None,
                ..next
            },
            (_, _) => next,
        }
    }

    fn create_settled(self, name: String, replaces: Option<String>, succeeded: bool) -> Self {
        let builder = match (succeeded, self.builder) {
            (true, Some(b)) if b.submitted => None,
            (false, Some(b)) if b.submitted => Some(b.with_submitted(false)),
            (_, other) => other,
        };
        let next = Self { builder, ..self };
        match (succeeded, replaces) {
            (true, Some(old)) => Self {
                drop_old: Some(DropOld {
                    old,
                    new: name.clone(),
                }),
                ..next.selecting(name)
            },
            (true, None) => next.selecting(name),
            (false, _) => next,
        }
    }

    /// Records what the write about to be sent is (see `intended`).
    pub fn intending(self, intended: Option<IndexWrite>) -> Self {
        Self { intended, ..self }
    }

    pub fn with_drop_confirm(self, drop_confirm: Option<DropConfirm>) -> Self {
        Self {
            drop_confirm,
            ..self
        }
    }

    pub fn with_drop_old(self, drop_old: Option<DropOld>) -> Self {
        Self { drop_old, ..self }
    }

    pub fn with_ttl_prompt(self, ttl_prompt: Option<TtlPrompt>) -> Self {
        Self { ttl_prompt, ..self }
    }

    /// Whether any modal of the write flows is open.
    pub fn has_prompt(&self) -> bool {
        self.builder.is_some()
            || self.drop_confirm.is_some()
            || self.drop_old.is_some()
            || self.ttl_prompt.is_some()
    }

    /// The builder modal open on `builder`, or closed with `None`.
    pub fn with_builder(self, builder: Option<IndexBuilderState>) -> Self {
        Self { builder, ..self }
    }
}

/// What the person did on the Indexes tab.
#[derive(Clone, Debug, PartialEq)]
pub enum IndexesAction {
    /// Open the index builder on the shown collection.
    OpenBuilder,
    /// Close the builder, keeping nothing.
    CancelBuilder,
    /// One change in the open builder.
    Builder(BuilderEdit),
    /// Create the index the builder describes.
    Create,
    /// The tab was opened: list the indexes and read their usage.
    OpenTab,
    Refresh,
    Select(String),
    SetRange(ChartRange),
    /// Copy the JSON of an index definition. Recorded nowhere in the state: the shell puts
    /// [`IndexesAction::clipboard_text`] on the clipboard.
    CopyJson(String),
    /// Open the builder pre-filled with a copy of this index; the original stays until the copy
    /// is built and the person agrees to drop it.
    EditCopy(String),
    /// Drop the index a copy replaced.
    ConfirmDropOld,
    /// Keep the index a copy replaced.
    KeepOld,
    /// Hide (`true`) or unhide an index from the query planner.
    SetHidden(String, bool),
    /// Open the TTL input for this index.
    ChangeTtl(String),
    TtlTyped(String),
    TtlCancelled,
    /// Change the expiry of this index, in seconds, in place.
    SetTtl(String, i64),
    /// Open the typed confirmation of dropping this index.
    DropRequested(String),
    DropTyped(String),
    /// Drop the index of the open confirmation.
    DropConfirmed,
    DropCancelled,
    /// Leave this database out of sampling (the name as typed; the reducer trims it).
    ExcludeDatabase(String),
    /// Sample this excluded database again.
    IncludeDatabase(String),
}

impl IndexesAction {
    /// The text this action puts on the clipboard, if it is a copy.
    pub fn clipboard_text(&self) -> Option<&str> {
        match self {
            Self::CopyJson(json) => Some(json),
            _ => None,
        }
    }
}
