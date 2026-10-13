//! What the results table and the detail pane offer for selecting rows and queueing writes: which
//! action buttons exist, why one is disabled, and which visible rows are ticked. Pure.
//!
//! Everything is a membership lookup for the rows on screen. The selection is never walked, so an
//! "all in category" selection of a million rows costs the same as one ticked row.
use curlybrace_core::compare::protocol::PairId;
use curlybrace_core::compare::results::Category;

use super::{CompareState, ResultsView, RowAction, Scope, Selection};

/// Why ticking or queueing is closed right now. Each variant has one `compare.apply.reason.*`
/// text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionBlock {
    /// An apply is being reviewed or runs: the lists are read-only.
    Busy,
    /// An apply touched the pair: its results are stale until it runs again.
    Applied,
    /// The pair has no finished results to act on.
    NotCompleted,
    /// A compare run is in flight.
    Running,
    /// A check is in flight.
    Checking,
    /// Nothing is ticked.
    NothingSelected,
}

impl ActionBlock {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Busy => "compare.apply.reason.busy",
            Self::Applied => "compare.apply.reason.applied",
            Self::NotCompleted => "compare.apply.reason.not_completed",
            Self::Running => "compare.apply.reason.running",
            Self::Checking => "compare.apply.reason.checking",
            Self::NothingSelected => "compare.apply.reason.nothing_selected",
        }
    }
}

/// One action button and why it is disabled (`None`: enabled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionButton {
    pub action: RowAction,
    pub block: Option<ActionBlock>,
}

impl CompareState {
    /// Why rows of `pair` cannot be ticked (the reducer refuses exactly these).
    pub fn tick_block(&self, pair: PairId) -> Option<ActionBlock> {
        if self.apply.is_busy() {
            return Some(ActionBlock::Busy);
        }
        if self.pair_is_applicable(pair) {
            return None;
        }
        Some(if self.pair_is_applied(pair) {
            ActionBlock::Applied
        } else {
            ActionBlock::NotCompleted
        })
    }

    /// Why an action on `selected` rows of `pair` cannot be queued; `None` when it can.
    pub fn queue_block(&self, pair: PairId, selected: usize) -> Option<ActionBlock> {
        if let Some(block) = self.tick_block(pair) {
            return Some(block);
        }
        if self.is_running() {
            Some(ActionBlock::Running)
        } else if self.check_in_flight() {
            Some(ActionBlock::Checking)
        } else if selected == 0 {
            Some(ActionBlock::NothingSelected)
        } else {
            None
        }
    }

    /// The pair's results are stale because an apply wrote to it.
    pub fn pair_is_applied(&self, pair: PairId) -> bool {
        self.overview
            .rows
            .iter()
            .any(|r| r.pair == pair && r.applied)
    }

    /// Documents queued for `pair` (a category item counts its whole tab).
    pub fn pending_documents(&self, pair: PairId) -> usize {
        self.pending
            .items()
            .iter()
            .filter(|c| c.pair == pair)
            .map(|c| match &c.scope {
                Scope::Keys(keys) => keys.len(),
                Scope::Category => self
                    .results
                    .get(&pair)
                    .map_or(0, |r| r.counts.of(c.category)),
            })
            .sum()
    }

    /// The action buttons for `selected` rows of a `category` tab of `pair`.
    pub fn action_buttons(
        &self,
        pair: PairId,
        category: Category,
        selected: usize,
    ) -> Vec<ActionButton> {
        let block = self.queue_block(pair, selected);
        RowAction::offered_for(category)
            .iter()
            .map(|&action| ActionButton { action, block })
            .collect()
    }

    /// The buttons for the one key open in the detail pane (it belongs to the open tab).
    pub fn detail_buttons(&self) -> Vec<ActionButton> {
        match self.open_results() {
            Some(r) => self.action_buttons(r.pair, r.category, 1),
            None => Vec::new(),
        }
    }
}

/// The selection bar above the table.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionBar {
    pub selected: usize,
    /// Rows of the tab, loaded or not.
    pub total: usize,
    /// Why the checkboxes and Select all are closed.
    pub tick_block: Option<ActionBlock>,
    /// Every row of the tab is selected through the "all" flag.
    pub all_selected: bool,
    pub actions: Vec<ActionButton>,
}

impl SelectionBar {
    pub fn of(state: &CompareState, results: &ResultsView) -> Self {
        let (pair, cat, total) = (results.pair, results.category, results.total());
        let selected = state.selection.count(pair, cat, total);
        Self {
            selected,
            total,
            tick_block: state.tick_block(pair),
            all_selected: state.selection.get(pair, cat).is_some_and(|e| e.all),
            actions: state.action_buttons(pair, cat, selected),
        }
    }

    pub fn can_select_all(&self) -> bool {
        self.tick_block.is_none() && self.total > 0 && !self.all_selected
    }

    pub fn can_clear(&self) -> bool {
        self.tick_block.is_none() && self.selected > 0
    }
}

/// Whether each visible row is ticked: one lookup per row, never a walk of the selection.
pub fn visible_ticks(selection: &Selection, results: &ResultsView) -> Vec<bool> {
    results
        .rows
        .iter()
        .map(|row| selection.is_selected(results.pair, results.category, &row.key))
        .collect()
}

/// The header checkbox: ticked when the loaded page is non-empty and every row of it is ticked.
pub fn page_ticked(ticks: &[bool]) -> bool {
    !ticks.is_empty() && ticks.iter().all(|t| *t)
}

#[cfg(test)]
#[path = "select_bar_tests.rs"]
mod tests;
