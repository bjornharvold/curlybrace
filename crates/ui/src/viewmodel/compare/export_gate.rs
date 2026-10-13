//! Which export buttons may fire, and why not. The reducer applies the same gates, so a click
//! the views allow is a click the reducer honours. Pure.
use curlybrace_core::compare::export::report::CLIPBOARD_MAX_ROWS;
use curlybrace_core::compare::protocol::{ExportFormat, PairId};

use super::CompareState;
use super::export::{ExportDraft, ExportKind};
use super::overview::PairStatus;
use super::pairs::EntryId;
use curlybrace_core::compare::apply::Direction;

/// Why an export button is disabled. The first reason that holds is the one shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportBlock {
    SessionNotReady,
    ExportBusy,
    ApplyBusy,
    RunActive,
    CheckInFlight,
    /// Plan: nothing is queued.
    EmptyPending,
    /// Report: no pair has finished results.
    NoCompletedPair,
    /// Report: none of the chosen pairs has finished results.
    NoPairChosen,
    /// Script: the queued writes land on both sides, which one script cannot target.
    MixedSides,
    /// Clipboard: the chosen pairs hold more differences than the clipboard takes.
    ClipboardTooLarge,
}

impl ExportBlock {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::SessionNotReady => "compare.export.reason.session_not_ready",
            Self::ExportBusy => "compare.export.reason.export_busy",
            Self::ApplyBusy => "compare.export.reason.apply_busy",
            Self::RunActive => "compare.export.reason.run_active",
            Self::CheckInFlight => "compare.export.reason.check_in_flight",
            Self::EmptyPending => "compare.export.reason.empty_pending",
            Self::NoCompletedPair => "compare.export.reason.no_completed_pair",
            Self::NoPairChosen => "compare.export.reason.no_pair_chosen",
            Self::MixedSides => "compare.export.reason.mixed_sides",
            Self::ClipboardTooLarge => "compare.export.reason.clipboard_too_large",
        }
    }
}

impl CompareState {
    /// Entries whose latest run completed and still holds its results, with the runtime pair,
    /// in entry order. A report covers only these.
    pub fn completed_entries(&self) -> Vec<(EntryId, PairId)> {
        let mut found: Vec<(EntryId, PairId)> = self
            .overview
            .rows
            .iter()
            .filter(|r| matches!(r.status, PairStatus::Completed(_)))
            .filter(|r| self.results.contains_key(&r.pair))
            .map(|r| (r.entry, r.pair))
            .collect();
        found.sort_unstable();
        found
    }

    /// Why `kind` cannot be opened now; `None` when it can.
    pub fn export_block(&self, kind: ExportKind) -> Option<ExportBlock> {
        self.base_block(kind, self.export.is_busy())
    }

    /// The gates shared by every export. `export_busy` is false when the caller is itself the
    /// export in progress (choosing a file, starting the job).
    fn base_block(&self, kind: ExportKind, export_busy: bool) -> Option<ExportBlock> {
        let reason = |holds: bool, block| holds.then_some(block);
        reason(!self.is_ready(), ExportBlock::SessionNotReady)
            .or(reason(export_busy, ExportBlock::ExportBusy))
            .or(reason(self.apply.is_busy(), ExportBlock::ApplyBusy))
            .or(reason(self.is_running(), ExportBlock::RunActive))
            .or(reason(self.check_in_flight(), ExportBlock::CheckInFlight))
            .or_else(|| self.kind_block(kind))
    }

    fn kind_block(&self, kind: ExportKind) -> Option<ExportBlock> {
        match kind {
            ExportKind::Plan if self.pending.is_empty() => Some(ExportBlock::EmptyPending),
            ExportKind::Report if self.completed_entries().is_empty() => {
                Some(ExportBlock::NoCompletedPair)
            }
            _ => None,
        }
    }

    /// The queued writes land on both sides (a script can only target one).
    pub fn pending_has_mixed_sides(&self) -> bool {
        let (ops, bulk) = self.pending.to_command_parts();
        let directions: Vec<Direction> = ops
            .iter()
            .map(|o| o.direction)
            .chain(bulk.iter().map(|b| b.direction))
            .collect();
        directions.iter().any(|d| Some(d) != directions.first())
    }

    /// The chosen entries that have finished results, with their runtime pairs.
    pub fn chosen_pairs(&self, draft: &ExportDraft) -> Vec<(EntryId, PairId)> {
        self.completed_entries()
            .into_iter()
            .filter(|(entry, _)| draft.report.pairs.contains(entry))
            .collect()
    }

    /// The differences the chosen pairs hold (the stored counts), as the clipboard cap counts.
    pub fn chosen_rows(&self, draft: &ExportDraft) -> usize {
        self.chosen_pairs(draft)
            .iter()
            .filter_map(|(_, pair)| self.results.get(pair))
            .map(|r| r.counts.only_in_source + r.counts.only_in_target + r.counts.different)
            .fold(0, usize::saturating_add)
    }

    /// Why the draft cannot go to a file now (choose a file, start the job); the export state
    /// itself is not counted as busy.
    pub fn file_block(&self, draft: &ExportDraft) -> Option<ExportBlock> {
        if let Some(block) = self.base_block(draft.kind, false) {
            return Some(block);
        }
        match draft.kind {
            ExportKind::Plan => (draft.plan.format == ExportFormat::Mongosh
                && self.pending_has_mixed_sides())
            .then_some(ExportBlock::MixedSides),
            ExportKind::Report => self
                .chosen_pairs(draft)
                .is_empty()
                .then_some(ExportBlock::NoPairChosen),
        }
    }

    /// Why the report cannot go to the clipboard now (a plan never does: `NoCompletedPair`).
    pub fn copy_block(&self, draft: &ExportDraft) -> Option<ExportBlock> {
        if draft.kind != ExportKind::Report {
            return Some(ExportBlock::NoCompletedPair);
        }
        if let Some(block) = self.file_block(draft) {
            return Some(block);
        }
        (self.chosen_rows(draft) > CLIPBOARD_MAX_ROWS).then_some(ExportBlock::ClipboardTooLarge)
    }

    /// The format a fresh plan modal starts on: the script, unless it is refused for these writes.
    pub fn default_plan_format(&self) -> ExportFormat {
        match self.pending_has_mixed_sides() {
            true => ExportFormat::JsonPlan,
            false => ExportFormat::Mongosh,
        }
    }
}
