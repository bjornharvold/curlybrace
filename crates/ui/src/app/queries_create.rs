//! Create index... on a row of the Queries view: the index builder opens pre-filled with the
//! index the shape's row suggests, through `index_write_actions::open_checked` (which refuses a
//! read-only connection, asks for the index list when it is not loaded and leaves production to
//! the builder's own write guard). Nothing is built without the builder's Create.
use super::index_write_actions::open_checked;
use super::reducer::Out;
use super::state::{AppState, Banner};
use crate::viewmodel::{IndexBuilderState, suggested_of};
use curlybrace_core::history::ShapeSource;

/// Opens the builder for the shape `hash` and `source` of the open Queries view. When the row is
/// gone or is no longer a miss, a banner says so instead. A shape carries no collation, so the
/// builder notes that the query may have used one.
pub(super) fn create_index(state: AppState, hash: &str, source: ShapeSource) -> Out {
    let make = |s: &AppState, ns: &curlybrace_core::store::Namespace| {
        // The rows belong to the collection the Indexes state holds, which must be the selected
        // one: a stale click never pre-fills a builder from another collection's shapes.
        (s.indexes.ns.as_ref() == Some(ns)).then_some(())?;
        let summary = s
            .indexes
            .queries
            .shapes
            .iter()
            .find(|shape| shape.hash == hash && shape.source == source)?;
        let index = suggested_of(summary)?;
        let builder = IndexBuilderState::from_suggested(ns.clone(), &index, None);
        Some(builder.with_collation_unrecorded())
    };
    open_checked(state, make, Some(Banner::IndexSuggestionGone))
}
