//! View-model of the Indexes tab: which tab of a collection is shown, and what the tab holds.
//! Pure and egui-free; the updaters return new values.
use super::index_builder::{BuilderEdit, IndexBuilderState};
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::{IndexSeries, SamplerStatus};
use curlybrace_core::store::{CollectionIndex, Namespace};

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
    /// The name of the index whose create was sent and not yet answered. It is what gets
    /// selected on success, whatever the builder (if any) holds by then.
    pub creating: Option<String>,
}

impl Default for IndexesState {
    fn default() -> Self {
        Self {
            ns: None,
            indexes: Vec::new(),
            series: Vec::new(),
            selected: None,
            range: ChartRange::Day,
            loading: false,
            sampler: None,
            builder: None,
            creating: None,
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
        Self {
            indexes,
            selected,
            loading: false,
            ..self
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

    /// Create of `name` on `ns` was sent: the builder open on `ns` is the one submitted.
    pub fn create_sent(self, ns: &Namespace, name: String) -> Self {
        let builder = self.builder.map(|b| {
            if b.ns == *ns {
                b.with_submitted(true)
            } else {
                b
            }
        });
        Self {
            builder,
            creating: Some(name),
            ..self
        }
    }

    /// The create was answered. On success the submitted builder (only that one) closes and its
    /// index is selected; on failure the builder is editable again.
    pub fn create_answered(self, succeeded: bool) -> Self {
        let created = self.creating.clone();
        let builder = match (succeeded, self.builder) {
            (true, Some(b)) if b.submitted => None,
            (false, Some(b)) if b.submitted => Some(b.with_submitted(false)),
            (_, other) => other,
        };
        let next = Self {
            builder,
            creating: None,
            ..self
        };
        match (succeeded, created) {
            (true, Some(name)) => next.selecting(name),
            _ => next,
        }
    }

    /// The builder modal open on `builder`, or closed with `None`.
    pub fn with_builder(self, builder: Option<IndexBuilderState>) -> Self {
        Self { builder, ..self }
    }
}

/// What the person did on the Indexes tab. Later tasks add the other write actions.
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
    /// Copy the JSON of an index definition. Recorded nowhere in the state; the clipboard action
    /// is wired by the task that adds the detail view.
    CopyJson(String),
}
