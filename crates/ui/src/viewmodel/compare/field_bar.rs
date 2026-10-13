//! What the detail pane offers for copying single fields: which checkbox is open (and the reason
//! when it is not), which of the two queue buttons is enabled, and the reason when it is not. Pure.
//!
//! The gates are the ones of the whole-document actions (`select_bar`): an apply under way, an
//! applied pair, a pair without finished results, a run or a check in flight. On top of them a
//! field is closed when core cannot write it in either direction, and a queue button when
//! nothing is ticked or no ticked field can be copied in its direction.
use curlybrace_core::compare::apply::Direction;
use curlybrace_core::compare::apply_fields::Unaddressable;
use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::protocol::PairId;

use super::{ActionBlock, ChangeLine, CompareState, DetailView, RowAction};

const DIRECTIONS: [Direction; 2] = [Direction::SourceToTarget, Direction::TargetToSource];

/// Why a field checkbox, a queue button or Clear is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldBlock {
    /// The gate of the whole-document actions.
    Gate(ActionBlock),
    /// The field cannot be written through its path, in either direction.
    Unaddressable(Unaddressable),
    /// No field is ticked.
    NoFields,
    /// Fields are ticked, but none of them can be copied in this direction.
    NotCopyableHere,
}

impl FieldBlock {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Gate(block) => block.i18n_key(),
            Self::Unaddressable(reason) => reason.i18n_key(),
            Self::NoFields => "compare.apply.reason.no_fields",
            Self::NotCopyableHere => "compare.apply.reason.not_copyable_here",
        }
    }
}

/// One line of the changes grid: its checkbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRow {
    pub ticked: bool,
    /// Why the checkbox is disabled; `None`: it can be clicked.
    pub block: Option<FieldBlock>,
}

/// The field controls of the open detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldBar {
    /// One per `DetailView::changes`, in order.
    pub rows: Vec<FieldRow>,
    /// Ticked fields of this document.
    pub selected: usize,
    pub to_target: Option<FieldBlock>,
    pub to_source: Option<FieldBlock>,
    pub clear: Option<FieldBlock>,
}

impl FieldBar {
    pub fn of(state: &CompareState, detail: &DetailView) -> Self {
        let pair = state.open_results().map(|r| r.pair);
        let ticked = |segments: &[PathSegment]| {
            pair.is_some_and(|p| state.field_selection.is_selected(p, &detail.key, segments))
        };
        Self {
            rows: detail
                .changes
                .iter()
                .map(|line| FieldRow {
                    ticked: ticked(&line.segments),
                    block: state.field_tick_block(detail, line),
                })
                .collect(),
            selected: pair.map_or(0, |p| state.field_selection.count(p, &detail.key)),
            to_target: state.field_queue_block(detail, Direction::SourceToTarget),
            to_source: state.field_queue_block(detail, Direction::TargetToSource),
            clear: state.field_clear_block(detail),
        }
    }

    /// Why `action` cannot be queued (`None`: it can).
    pub fn queue_block(&self, action: RowAction) -> Option<FieldBlock> {
        match action.direction() {
            Direction::SourceToTarget => self.to_target,
            Direction::TargetToSource => self.to_source,
        }
    }
}

impl CompareState {
    /// The pair whose results the detail belongs to when fields may be acted on: ticking is
    /// open for the pair and the detail's key is a row of the loaded page.
    fn field_gate(&self, detail: &DetailView) -> Result<PairId, ActionBlock> {
        let Some(results) = self.open_results() else {
            return Err(ActionBlock::NotCompleted);
        };
        if let Some(block) = self.tick_block(results.pair) {
            return Err(block);
        }
        match self.key_on_open_page(&detail.key) {
            true => Ok(results.pair),
            false => Err(ActionBlock::NotOnPage),
        }
    }

    /// Why the checkbox of `line` is disabled; `None` when it can be ticked.
    pub fn field_tick_block(&self, detail: &DetailView, line: &ChangeLine) -> Option<FieldBlock> {
        if let Err(block) = self.field_gate(detail) {
            return Some(FieldBlock::Gate(block));
        }
        let reasons: Vec<_> = DIRECTIONS.iter().map(|d| line.copy_block(*d)).collect();
        match reasons.iter().all(Option::is_some) {
            true => reasons
                .first()
                .copied()
                .flatten()
                .map(FieldBlock::Unaddressable),
            false => None,
        }
    }

    /// Why the ticked fields of `detail` cannot be queued in `direction`; `None` when they can.
    pub fn field_queue_block(
        &self,
        detail: &DetailView,
        direction: Direction,
    ) -> Option<FieldBlock> {
        let pair = match self.field_gate(detail) {
            Ok(pair) => pair,
            Err(block) => return Some(FieldBlock::Gate(block)),
        };
        if let Some(block) = self.queue_block(pair, 1) {
            return Some(FieldBlock::Gate(block));
        }
        let ticked = self.ticked_lines(pair, detail);
        if ticked.is_empty() {
            return Some(FieldBlock::NoFields);
        }
        let any = ticked.iter().any(|l| l.copy_block(direction).is_none());
        (!any).then_some(FieldBlock::NotCopyableHere)
    }

    /// Why Clear is closed.
    pub fn field_clear_block(&self, detail: &DetailView) -> Option<FieldBlock> {
        let pair = match self.field_gate(detail) {
            Ok(pair) => pair,
            Err(block) => return Some(FieldBlock::Gate(block)),
        };
        (self.field_selection.count(pair, &detail.key) == 0).then_some(FieldBlock::NoFields)
    }

    /// The detail's lines that are ticked.
    fn ticked_lines<'a>(&self, pair: PairId, detail: &'a DetailView) -> Vec<&'a ChangeLine> {
        detail
            .changes
            .iter()
            .filter(|l| {
                self.field_selection
                    .is_selected(pair, &detail.key, &l.segments)
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "field_bar_tests.rs"]
mod tests;
