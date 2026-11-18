//! Create index... on a row of the Queries view: the index builder opens pre-filled with the
//! index the shape's row suggests, through `index_write_actions::open_with` (which refuses a
//! read-only connection and leaves production to the builder's own write guard). Nothing is built
//! without the builder's Create.
use super::index_write_actions::open_with;
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::{IndexBuilderState, rows};

/// Opens the builder for the shape `hash` of the open Queries view; nothing when the shape is not
/// there or is no longer a miss. No collation: a shape recorded from the app or the profiler
/// carries none.
pub(super) fn create_index(state: AppState, hash: &str) -> Out {
    open_with(state, |s, ns| {
        let row = rows(&s.indexes.queries, ns)
            .into_iter()
            .find(|row| row.hash == hash)?;
        let index = row.suggested?;
        Some(IndexBuilderState::from_suggested(ns.clone(), &index, None))
    })
}
