//! What the Compare tables show per row: the pair list, the preflight verdicts and the run
//! overview. Every mapping (status to key and icon, effective key, the marker for a customised
//! pair, which row actions are live) is decided here; the views only draw it. Pure.
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::Namespace;

use super::CompareState;
use super::checks::EntryCheck;
use super::digest::PairsDigest;
use super::overview::{PairStatus, Totals};
use super::pairs::{EntryId, PairEntry};
use super::preflight::{PreflightView, side_key};
use super::setup::SetupError;

/// A status symbol. Always drawn next to its label, so colour is never the only carrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusIcon {
    Ok,
    Warning,
    Failed,
    Pending,
    Running,
    Cancelled,
    Discarded,
    Unchecked,
    /// The pair differs from the defaults.
    Customised,
}

impl StatusIcon {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Ok => "✔",
            Self::Warning => "⚠",
            Self::Failed => "✖",
            Self::Pending => "⏳",
            Self::Running => "▶",
            Self::Cancelled => "⏹",
            Self::Discarded => "—",
            Self::Unchecked => "?",
            Self::Customised => "≠",
        }
    }
}

/// The outcome of one entry's preflight, or why it cannot be checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryStatus {
    /// The entry's settings do not build; the issue says why.
    Invalid(SetupError),
    Unchecked,
    Checking,
    Ok,
    NeedsIndex(Side),
    EmptyBoth,
    CheckFailed,
}

impl EntryStatus {
    fn of(issue: Option<SetupError>, check: Option<&EntryCheck>) -> Self {
        if let Some(issue) = issue {
            return Self::Invalid(issue);
        }
        match check {
            Some(EntryCheck::Queued | EntryCheck::Checking(_)) => Self::Checking,
            Some(EntryCheck::Failed) => Self::CheckFailed,
            Some(EntryCheck::Done(view, _)) => Self::from_verdict(*view),
            Some(EntryCheck::Unchecked) | None => Self::Unchecked,
        }
    }

    fn from_verdict(view: PreflightView) -> Self {
        match view {
            PreflightView::Unchecked => Self::Unchecked,
            PreflightView::Checking => Self::Checking,
            PreflightView::Ok => Self::Ok,
            PreflightView::NeedsIndex { side } => Self::NeedsIndex(side),
            PreflightView::EmptyBoth => Self::EmptyBoth,
        }
    }

    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "compare.pairs.invalid",
            Self::Unchecked => "compare.preflight.entry_unchecked",
            Self::Checking => "compare.preflight.entry_checking",
            Self::Ok => "compare.preflight.entry_ok",
            Self::NeedsIndex(_) => "compare.preflight.entry_needs_index",
            Self::EmptyBoth => "compare.preflight.entry_empty",
            Self::CheckFailed => "compare.preflight.entry_failed",
        }
    }

    /// The translated side name substituted as `{side}`, when the label names one.
    pub fn side_key(&self) -> Option<&'static str> {
        match self {
            Self::NeedsIndex(side) => Some(side_key(*side)),
            _ => None,
        }
    }

    /// The longer explanation shown on hover (the setup issue behind an invalid entry).
    pub fn hover_key(&self) -> Option<&'static str> {
        match self {
            Self::Invalid(issue) => Some(issue.i18n_key()),
            _ => None,
        }
    }

    pub fn icon(&self) -> StatusIcon {
        match self {
            Self::Ok | Self::EmptyBoth => StatusIcon::Ok,
            Self::Invalid(_) => StatusIcon::Warning,
            Self::NeedsIndex(_) | Self::CheckFailed => StatusIcon::Failed,
            Self::Unchecked => StatusIcon::Unchecked,
            Self::Checking => StatusIcon::Pending,
        }
    }
}

/// `db.collection`.
pub fn namespace_label(ns: &Namespace) -> String {
    format!("{}.{}", ns.db, ns.collection)
}

/// One line of the pair table (and of the preflight list).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairRow {
    pub entry: EntryId,
    pub included: bool,
    pub source: String,
    pub target: String,
    /// The key in effect: the pair's own, else the default.
    pub key: String,
    pub customised: bool,
    pub status: EntryStatus,
}

impl PairRow {
    fn of(
        entry: &PairEntry,
        defaults_key: &str,
        issue: Option<SetupError>,
        status: EntryStatus,
    ) -> Self {
        debug_assert!(issue.is_none() || matches!(status, EntryStatus::Invalid(_)));
        Self {
            entry: entry.id,
            included: entry.included,
            source: namespace_label(&entry.source),
            target: namespace_label(&entry.target),
            key: entry
                .overrides
                .key_text
                .clone()
                .unwrap_or_else(|| defaults_key.to_string()),
            customised: entry.overrides.is_customised(),
            status,
        }
    }
}

/// The four category counts of a finished pair, or the sums over all finished pairs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowCounts {
    pub different: u64,
    pub only_in_source: u64,
    pub only_in_target: u64,
    pub identical: u64,
}

impl From<&Summary> for RowCounts {
    fn from(s: &Summary) -> Self {
        Self {
            different: s.different,
            only_in_source: s.only_in_source,
            only_in_target: s.only_in_target,
            identical: s.equal,
        }
    }
}

impl From<&Totals> for RowCounts {
    fn from(t: &Totals) -> Self {
        Self {
            different: t.different,
            only_in_source: t.only_in_source,
            only_in_target: t.only_in_target,
            identical: t.identical,
        }
    }
}

/// What a run row says beyond its status.
#[derive(Clone, Debug, PartialEq)]
pub enum RowDetail {
    None,
    Progress {
        scanned_source: u64,
        scanned_target: u64,
        differences: u64,
    },
    Counts(RowCounts),
    Error(CoreError),
}

impl RowDetail {
    fn of(status: &PairStatus) -> Self {
        match status {
            PairStatus::Running {
                scanned_source,
                scanned_target,
                differences,
            } => Self::Progress {
                scanned_source: *scanned_source,
                scanned_target: *scanned_target,
                differences: *differences,
            },
            PairStatus::Completed(s) | PairStatus::Cancelled(s) => Self::Counts(s.into()),
            PairStatus::Failed(e) => Self::Error(e.clone()),
            PairStatus::Pending | PairStatus::Discarded => Self::None,
        }
    }
}

/// One line of the run overview with the actions that are live for it.
#[derive(Clone, Debug, PartialEq)]
pub struct OverviewRow {
    pub entry: EntryId,
    pub source: String,
    pub target: String,
    pub status_key: &'static str,
    pub icon: StatusIcon,
    pub detail: RowDetail,
    pub can_open: bool,
    pub can_discard: bool,
    pub can_rerun: bool,
    /// Find duplicate keys is shown (the pair failed on a duplicate key and its panel is not
    /// already offering Try again)...
    pub offers_duplicates: bool,
    /// ...and live.
    pub can_find_duplicates: bool,
}

impl CompareState {
    /// The pair table line at `index` of the list; `digest` is this state's digest.
    pub fn pair_row(&self, index: usize, digest: &PairsDigest) -> Option<PairRow> {
        let entry = self.pairs.entries.get(index)?;
        let issue = digest.issue_at(index);
        let status = EntryStatus::of(issue, self.checks.entries.get(&entry.id));
        Some(PairRow::of(entry, &self.setup.key_text, issue, status))
    }

    /// The names of the open pair's entry (source, target); none when nothing is open or the
    /// entry is gone.
    pub fn open_pair_labels(&self) -> Option<(String, String)> {
        let pair = self.open_pair?;
        let row = self.overview.rows.iter().find(|r| r.pair == pair)?;
        self.entry_labels(row.entry)
    }

    /// The names (source, target) of an entry; none when it is gone.
    pub fn entry_labels(&self, entry: EntryId) -> Option<(String, String)> {
        let entry = self.pairs.get(entry)?;
        Some((
            namespace_label(&entry.source),
            namespace_label(&entry.target),
        ))
    }

    /// The issue of the pair being edited, if any (shown in its editor).
    pub fn editor_issue(&self) -> Option<SetupError> {
        let entry = self.pairs.get(self.pairs.editing?)?;
        entry
            .issues(&self.setup, self.one_connection())
            .into_iter()
            .next()
    }

    /// The overview line at `index` of the run's rows. None when the row's entry is gone.
    pub fn overview_row(&self, index: usize) -> Option<OverviewRow> {
        let row = self.overview.rows.get(index)?;
        let entry = self.pairs.get(row.entry)?;
        let status = &row.status;
        let live = matches!(status, PairStatus::Pending | PairStatus::Running { .. });
        Some(OverviewRow {
            entry: row.entry,
            source: namespace_label(&entry.source),
            target: namespace_label(&entry.target),
            status_key: status.i18n_key(),
            icon: status.icon(),
            detail: RowDetail::of(status),
            can_open: matches!(status, PairStatus::Completed(_))
                && self.results.contains_key(&row.pair),
            can_discard: status.is_settled(),
            can_rerun: !live,
            offers_duplicates: self.failed_on_duplicate_key(row.entry)
                && !self.duplicates_failed_for(row.entry),
            can_find_duplicates: self.can_find_duplicates_for(row.entry),
        })
    }
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
