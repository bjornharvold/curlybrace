//! The differences of a finished pair: three category counts, the identical count and one page
//! of rows.
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{CompareCommand, PairId, RowView, SessionId};
use curlybrace_core::compare::results::Category;

use super::detail::value_text;

pub const PAGE_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub only_in_source: usize,
    pub only_in_target: usize,
    pub different: usize,
    /// Matched documents that are equal under the rules: shown as a number, never listed.
    pub identical: u64,
}

impl Counts {
    pub fn of(&self, category: Category) -> usize {
        match category {
            Category::OnlyInSource => self.only_in_source,
            Category::OnlyInTarget => self.only_in_target,
            Category::Different => self.different,
        }
    }

    fn with(self, category: Category, n: usize) -> Self {
        match category {
            Category::OnlyInSource => Self {
                only_in_source: n,
                ..self
            },
            Category::OnlyInTarget => Self {
                only_in_target: n,
                ..self
            },
            Category::Different => Self {
                different: n,
                ..self
            },
        }
    }
}

pub fn category_key(category: Category) -> &'static str {
    match category {
        Category::OnlyInSource => "compare.category.only_in_source",
        Category::OnlyInTarget => "compare.category.only_in_target",
        Category::Different => "compare.category.different",
    }
}

const DISPLAY_ORDER: [Category; 3] = [
    Category::Different,
    Category::OnlyInSource,
    Category::OnlyInTarget,
];

fn count(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResultsView {
    /// The runtime pair these results belong to.
    pub pair: PairId,
    pub counts: Counts,
    pub category: Category,
    pub offset: usize,
    pub rows: Vec<RowView>,
    /// The page asked for and not yet delivered; any other `Rows` answer is stale.
    pub requested: Option<(Category, usize)>,
    /// Loading `requested` failed; it is kept so the user can retry it.
    pub load_failed: bool,
}

impl ResultsView {
    /// Starts on the first category that has differences and asks for its first page.
    pub fn new(pair: PairId, summary: &Summary) -> Self {
        let counts = Counts {
            only_in_source: count(summary.only_in_source),
            only_in_target: count(summary.only_in_target),
            different: count(summary.different),
            identical: summary.equal,
        };
        let category = DISPLAY_ORDER
            .into_iter()
            .find(|c| counts.of(*c) > 0)
            .unwrap_or(Category::Different);
        Self {
            pair,
            counts,
            category,
            offset: 0,
            rows: Vec::new(),
            requested: (counts.of(category) > 0).then_some((category, 0)),
            load_failed: false,
        }
    }

    pub fn total(&self) -> usize {
        self.counts.of(self.category)
    }

    pub fn is_loading(&self) -> bool {
        self.requested.is_some() && !self.load_failed
    }

    /// Every category is empty: the two collections match under the rules.
    pub fn is_clean(&self) -> bool {
        DISPLAY_ORDER.into_iter().all(|c| self.counts.of(c) == 0)
    }

    /// The outstanding page failed to load (a no-op when nothing is outstanding).
    pub fn rows_failed(self) -> Self {
        Self {
            load_failed: self.requested.is_some(),
            ..self
        }
    }

    /// Asks for the failed page again; `None` when nothing failed.
    pub fn retried(&self) -> Option<Self> {
        self.load_failed.then(|| Self {
            load_failed: false,
            ..self.clone()
        })
    }

    pub fn has_prev(&self) -> bool {
        self.offset > 0
    }

    pub fn has_next(&self) -> bool {
        self.offset + PAGE_SIZE < self.total()
    }

    /// The 1-based `(first, last)` positions of the current page; `(0, 0)` when empty.
    pub fn shown_range(&self) -> (usize, usize) {
        let total = self.total();
        if total == 0 {
            return (0, 0);
        }
        (self.offset + 1, (self.offset + PAGE_SIZE).min(total))
    }

    /// The `Rows` command for the outstanding request, if any.
    pub fn request(&self, session: SessionId) -> Option<CompareCommand> {
        self.requested
            .map(|(category, offset)| CompareCommand::Rows {
                session,
                pair: self.pair,
                category,
                offset,
                limit: PAGE_SIZE,
            })
    }

    /// `delta` pages forward or back, clamped to the pages that exist; `None` when nothing moves.
    pub fn paged(&self, delta: i64) -> Option<Self> {
        let total = self.total();
        if total == 0 {
            return None;
        }
        let last_page_start = ((total - 1) / PAGE_SIZE) * PAGE_SIZE;
        let step = delta.saturating_mul(PAGE_SIZE as i64);
        let wanted = (self.offset as i64).saturating_add(step).max(0);
        let offset = usize::try_from(wanted)
            .unwrap_or(usize::MAX)
            .min(last_page_start);
        (offset != self.offset).then(|| self.moved_to(self.category, offset))
    }

    /// Switches category (first page); `None` when it is already shown.
    pub fn in_category(&self, category: Category) -> Option<Self> {
        (category != self.category).then(|| self.moved_to(category, 0))
    }

    fn moved_to(&self, category: Category, offset: usize) -> Self {
        Self {
            category,
            offset,
            rows: Vec::new(),
            requested: (self.counts.of(category) > 0).then_some((category, offset)),
            load_failed: false,
            ..self.clone()
        }
    }

    /// Takes the answer only if it is the page that was asked for. Rows past `total` are cut.
    pub fn accept_rows(
        self,
        category: Category,
        offset: usize,
        total: usize,
        rows: Vec<RowView>,
    ) -> Self {
        if self.requested != Some((category, offset)) {
            return self;
        }
        let room = total.saturating_sub(offset).min(PAGE_SIZE);
        Self {
            counts: self.counts.with(category, total),
            rows: rows.into_iter().take(room).collect(),
            requested: None,
            load_failed: false,
            ..self
        }
    }

    /// The detail request for a key on the current page; `None` for any other key.
    pub fn select(&self, session: SessionId, key: &KeyTuple) -> Option<CompareCommand> {
        self.rows
            .iter()
            .any(|r| r.key == *key)
            .then(|| CompareCommand::Detail {
                session,
                pair: self.pair,
                key: key.clone(),
            })
    }
}

/// A key as compact text for a row label: one relaxed-JSON value per key field.
pub fn key_label(key: &KeyTuple) -> String {
    key.0.iter().map(value_text).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
#[path = "results_tests.rs"]
mod tests;
