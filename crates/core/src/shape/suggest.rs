//! Index suggestion from a shape's operators.

use bson::Document;

use super::Shape;
use super::normalize::to_document;
use crate::explain::suggest::suggest_from;
use crate::explain::{FieldOrderFinding, SuggestedIndex, check_field_order};

/// The first field-order problem of the index with `keys` for the query `shape` describes. The
/// shape holds `"?"` for every value, which classifies as an exact match, and operator documents
/// whose operators classify the range fields, so it judges as the real filter would.
pub(crate) fn field_order_for_shape(shape: &Shape, keys: &Document) -> Option<FieldOrderFinding> {
    let filter = shape.filter().map(to_document).unwrap_or_default();
    let sort = shape.sort().map(to_document);
    check_field_order(keys, &filter, sort.as_ref())
}

/// The equality, sort, range rule of `explain::suggest`, from the operators in the shape.
/// Equality fields come in the shape's (alphabetical) filter order, not the author's.
pub fn suggest_for_shape(shape: &Shape) -> Option<SuggestedIndex> {
    let filter = shape.filter().map(to_document).unwrap_or_default();
    let sort = shape.sort().map(to_document);
    suggest_from(&filter, sort.as_ref())
}
