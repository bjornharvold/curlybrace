//! Create index... on a row of the Queries view: the index builder opens pre-filled with the
//! index the shape's row suggests, through `index_write_actions::open_with` (which refuses a
//! read-only connection, asks for the index list when it is not loaded and leaves production to
//! the builder's own write guard). Nothing is built without the builder's Create.
use super::index_write_actions::open_with;
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::{IndexBuilderState, rows};
use curlybrace_core::history::ShapeSource;

/// Opens the builder for the row of shape `hash` and `source` of the open Queries view; nothing
/// when the row is not there or is no longer a miss. No collation: a shape recorded from the app
/// or the profiler carries none.
pub(super) fn create_index(state: AppState, hash: &str, source: ShapeSource) -> Out {
    open_with(state, |s, ns| {
        // The rows belong to the collection the Indexes state holds, which must be the selected
        // one: a stale click never pre-fills a builder from another collection's shapes.
        (s.indexes.ns.as_ref() == Some(ns)).then_some(())?;
        let row = rows(&s.indexes.queries, ns)
            .into_iter()
            .find(|row| row.hash == hash && row.source == source)?;
        let index = row.suggested?;
        Some(IndexBuilderState::from_suggested(ns.clone(), &index, None))
    })
}
