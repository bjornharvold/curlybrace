//! Index suggestion from a shape's operators.

use super::Shape;
use super::normalize::to_document;
use crate::explain::SuggestedIndex;
use crate::explain::suggest::suggest_from;

/// The equality, sort, range rule of `explain::suggest`, from the operators in the shape.
/// Equality fields come in the shape's (alphabetical) filter order, not the author's.
pub fn suggest_for_shape(shape: &Shape) -> Option<SuggestedIndex> {
    let filter = shape.filter().map(to_document).unwrap_or_default();
    let sort = shape.sort().map(to_document);
    suggest_from(&filter, sort.as_ref())
}
