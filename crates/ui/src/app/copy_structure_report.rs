//! What the report says about collection options and indexes: per collection, whether its target
//! was created with the source's options, how many indexes were built, left alone because the
//! target already had one of that name, or failed. Holds names, counts and message keys only; a
//! failed index is shown by its NAME and a typed reason, never by server text or a value.
use super::copy_state::{CopyRun, RunOutcome};
use curlybrace_core::copy::structure::{IndexFailReason, StructureOutcome};

/// The outcome of the options of one target collection, as a message key.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionsLine {
    pub collection: String,
    pub key: &'static str,
}

/// The index counts of one target collection.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexLine {
    pub collection: String,
    pub built: usize,
    pub existing: usize,
    pub failed: usize,
}

/// An index that could not be built: its name and a message key for the reason.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexFailureRow {
    pub collection: String,
    pub name: String,
    pub reason_key: &'static str,
}

pub fn options_key(outcome: StructureOutcome) -> Option<&'static str> {
    match outcome {
        StructureOutcome::Created => Some("copy.report.options_created"),
        StructureOutcome::ExistingKept => Some("copy.report.options_kept"),
        StructureOutcome::SourceNotPlain => Some("copy.report.options_not_plain"),
        StructureOutcome::SourceMissing => None,
    }
}

/// The message key of a failed index build. No server code or text is ever shown.
pub fn index_failure_key(reason: IndexFailReason) -> &'static str {
    match reason {
        IndexFailReason::DuplicateData => "copy.index_failure.duplicate_data",
        IndexFailReason::Unsupported => "copy.index_failure.unsupported",
        IndexFailReason::NotAllowed => "copy.index_failure.not_allowed",
        IndexFailReason::ConnectionLost => "copy.index_failure.connection_lost",
        IndexFailReason::Rejected => "copy.index_failure.rejected",
    }
}

impl CopyRun {
    /// Options and indexes were asked for.
    pub fn with_structure_requested(self, structure_requested: bool) -> Self {
        Self {
            structure_requested,
            ..self
        }
    }

    pub fn options_lines(&self) -> Vec<OptionsLine> {
        self.rows
            .iter()
            .filter_map(|row| {
                let key = options_key(row.structure?.outcome)?;
                Some(OptionsLine {
                    collection: row.target.collection.clone(),
                    key,
                })
            })
            .collect()
    }

    pub fn index_lines(&self) -> Vec<IndexLine> {
        self.rows
            .iter()
            .filter_map(|row| {
                let indexes = row.indexes.as_ref()?;
                Some(IndexLine {
                    collection: row.target.collection.clone(),
                    built: indexes.built.len(),
                    existing: indexes.existing.len(),
                    failed: indexes.failed.len(),
                })
            })
            .collect()
    }

    pub fn index_failures(&self) -> Vec<IndexFailureRow> {
        self.rows
            .iter()
            .flat_map(|row| {
                let failed = row.indexes.iter().flat_map(|i| i.failed.iter());
                failed.map(|f| IndexFailureRow {
                    collection: row.target.collection.clone(),
                    name: f.name.clone(),
                    reason_key: index_failure_key(f.reason),
                })
            })
            .collect()
    }

    /// The run ended early with the option on: the indexes of the collection it stopped in (and
    /// of those not reached) were not built.
    pub fn indexes_skipped(&self) -> bool {
        let ended_early = matches!(
            self.outcome,
            Some(RunOutcome::Cancelled | RunOutcome::Failed(_))
        );
        self.structure_requested && ended_early
    }
}
