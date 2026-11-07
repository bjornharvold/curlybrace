//! Index suggestion from a shape's operators.

use bson::{Bson, Document};
use serde_json::{Map, Value};

use super::Shape;
use crate::explain::SuggestedIndex;
use crate::explain::suggest::suggest_from;

fn to_bson(value: &Value) -> Bson {
    match value {
        Value::Object(map) => Bson::Document(to_document(map)),
        Value::Array(items) => Bson::Array(items.iter().map(to_bson).collect()),
        Value::String(s) => Bson::String(s.clone()),
        Value::Number(n) => n.as_i64().map_or(Bson::Null, Bson::Int64),
        Value::Bool(b) => Bson::Boolean(*b),
        Value::Null => Bson::Null,
    }
}

fn to_document(map: &Map<String, Value>) -> Document {
    map.iter().map(|(k, v)| (k.clone(), to_bson(v))).collect()
}

/// The equality, sort, range rule of `explain::suggest`, from the operators in the shape.
/// Equality fields come in the shape's (alphabetical) filter order, not the author's.
pub fn suggest_for_shape(shape: &Shape) -> Option<SuggestedIndex> {
    let filter = shape.filter().map(to_document).unwrap_or_default();
    let sort = shape.sort().map(to_document);
    suggest_from(&filter, sort.as_ref())
}
