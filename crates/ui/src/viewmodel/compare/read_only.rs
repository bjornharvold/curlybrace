//! Read-only destinations in Data Compare. A read-only connection may be compared on either side
//! but never receives a write, so every write is judged by the side it lands on: the connection
//! a direction writes to, whether that connection is read-only (the snapshot the session opened
//! with, raised by a lock, OR the saved connection's live flag, see
//! [`CompareState::is_read_only`]), and the queue actions and pending list judged by it. Pure.
use curlybrace_core::compare::apply::Direction;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::connection::ConnectionConfig;
use uuid::Uuid;

use super::{ActionBlock, ActionButton, CompareState, FieldBar, SelectionBar};

const DIRECTIONS: [(Direction, Side); 2] = [
    (Direction::SourceToTarget, Side::Target),
    (Direction::TargetToSource, Side::Source),
];

impl CompareState {
    /// The connection of the open session that a write in `direction` lands on.
    pub fn destination_connection(&self, direction: Direction) -> Option<Uuid> {
        self.connections.map(|(source, target)| match direction {
            Direction::SourceToTarget => target,
            Direction::TargetToSource => source,
        })
    }

    /// Whether a write in `direction` would land on a read-only connection.
    pub fn writes_to_read_only(&self, direction: Direction, saved: &[ConnectionConfig]) -> bool {
        self.destination_connection(direction)
            .is_some_and(|id| self.is_read_only(id, saved))
    }

    /// The read-only side some queued change writes to, the target first; `None` when every
    /// destination of the pending list is writable.
    pub fn read_only_destination(&self, saved: &[ConnectionConfig]) -> Option<Side> {
        DIRECTIONS.iter().find_map(|(direction, side)| {
            let queued = self
                .pending
                .items()
                .iter()
                .any(|c| c.action.direction() == *direction);
            (queued && self.writes_to_read_only(*direction, saved)).then_some(*side)
        })
    }

    /// Every reason Review changes is closed, in the order the person can clear them: the
    /// workflow block first (busy, running, checking, ...), then the read-only destination. Empty
    /// when Review can fire.
    pub fn review_reasons(&self, saved: &[ConnectionConfig]) -> Vec<ReviewReason> {
        let block = self.review_block().map(ReviewReason::Block);
        let read_only = self
            .read_only_destination(saved)
            .map(ReviewReason::ReadOnly);
        block.into_iter().chain(read_only).collect()
    }

    /// `buttons` with every action that writes to a read-only connection marked as such. The
    /// button's own block stays, so both reasons are stated.
    pub fn guard_buttons(
        &self,
        buttons: Vec<ActionButton>,
        saved: &[ConnectionConfig],
    ) -> Vec<ActionButton> {
        buttons
            .into_iter()
            .map(|button| ActionButton {
                read_only: self.writes_to_read_only(button.action.direction(), saved),
                ..button
            })
            .collect()
    }

    /// The selection bar with its read-only destinations marked.
    pub fn guard_selection_bar(
        &self,
        bar: SelectionBar,
        saved: &[ConnectionConfig],
    ) -> SelectionBar {
        SelectionBar {
            actions: self.guard_buttons(bar.actions, saved),
            ..bar
        }
    }

    /// The field bar with the queue button of a read-only destination marked.
    pub fn guard_field_bar(&self, bar: FieldBar, saved: &[ConnectionConfig]) -> FieldBar {
        FieldBar {
            to_target_read_only: self.writes_to_read_only(Direction::SourceToTarget, saved),
            to_source_read_only: self.writes_to_read_only(Direction::TargetToSource, saved),
            ..bar
        }
    }
}

/// The text shown when a queued or offered write goes to a read-only connection.
pub const READ_ONLY_DESTINATION_KEY: &str = "compare.apply.reason.read_only_destination";

/// One reason Review changes is closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewReason {
    /// The workflow (an apply, a run, a check, ...) holds it.
    Block(ActionBlock),
    /// A queued change writes to this read-only side.
    ReadOnly(Side),
}

impl ReviewReason {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Block(block) => block.i18n_key(),
            Self::ReadOnly(Side::Target) => "compare.apply.reason.read_only_target",
            Self::ReadOnly(Side::Source) => "compare.apply.reason.read_only_source",
        }
    }
}

#[cfg(test)]
#[path = "read_only_tests.rs"]
mod tests;
