//! Read-only destinations in Data Compare. A read-only connection may be compared on either side
//! but never receives a write, so every write is judged by the side it lands on: the connection
//! a direction writes to, whether that connection is read-only (the snapshot the session opened
//! with, raised by a lock, OR the saved connection's live flag, see
//! [`CompareState::is_read_only`]), and the queue actions and pending list judged by it. Pure.
use curlybrace_core::compare::apply::Direction;
use curlybrace_core::compare::protocol::Side;
use curlybrace_core::connection::ConnectionConfig;
use uuid::Uuid;

use super::{ActionBlock, ActionButton, CompareState, FieldBar, FieldBlock, SelectionBar};

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

    /// `buttons` with every action that writes to a read-only connection blocked for that reason.
    pub fn guard_buttons(
        &self,
        buttons: Vec<ActionButton>,
        saved: &[ConnectionConfig],
    ) -> Vec<ActionButton> {
        buttons
            .into_iter()
            .map(
                |button| match self.writes_to_read_only(button.action.direction(), saved) {
                    true => ActionButton {
                        block: Some(ActionBlock::ReadOnlyDestination),
                        ..button
                    },
                    false => button,
                },
            )
            .collect()
    }

    /// The selection bar with its read-only destinations blocked.
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

    /// The field bar with the queue button of a read-only destination blocked.
    pub fn guard_field_bar(&self, bar: FieldBar, saved: &[ConnectionConfig]) -> FieldBar {
        let guard = |direction: Direction, block: Option<FieldBlock>| match self
            .writes_to_read_only(direction, saved)
        {
            true => Some(FieldBlock::Gate(ActionBlock::ReadOnlyDestination)),
            false => block,
        };
        FieldBar {
            to_target: guard(Direction::SourceToTarget, bar.to_target),
            to_source: guard(Direction::TargetToSource, bar.to_source),
            ..bar
        }
    }
}

#[cfg(test)]
#[path = "read_only_tests.rs"]
mod tests;
