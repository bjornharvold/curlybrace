//! The equality, sort, range rule for proposing an index.

use bson::{Bson, Document};

use super::verdict::SuggestedIndex;
use crate::store::Query;

const RANGE_OPS: [&str; 6] = ["$gt", "$gte", "$lt", "$lte", "$ne", "$regex"];
const EQUALITY_OPS: [&str; 2] = ["$eq", "$in"];

fn operators(value: &Bson) -> Option<Vec<&str>> {
    match value {
        Bson::Document(d) if d.keys().all(|k| k.starts_with('$')) && !d.is_empty() => {
            Some(d.keys().map(String::as_str).collect())
        }
        _ => None,
    }
}

fn is_equality(value: &Bson) -> bool {
    operators(value).is_none_or(|ops| ops.iter().all(|o| EQUALITY_OPS.contains(o)))
}

fn is_range(value: &Bson) -> bool {
    operators(value).is_some_and(|ops| ops.iter().any(|o| RANGE_OPS.contains(o)))
}

fn direction(value: &Bson) -> i32 {
    let negative = match value {
        Bson::Int32(n) => *n < 0,
        Bson::Int64(n) => *n < 0,
        Bson::Double(n) => *n < 0.0,
        _ => false,
    };
    if negative { -1 } else { 1 }
}

fn filter_fields(filter: &Document, keep: fn(&Bson) -> bool) -> Vec<(String, i32)> {
    filter
        .iter()
        .filter(|(k, v)| !k.starts_with('$') && keep(v))
        .map(|(k, _)| (k.clone(), 1))
        .collect()
}

/// Equality fields (filter order), then sort fields (direction kept), then range
/// fields; duplicates dropped. `None` when there is no filter and no sort.
pub fn suggest(query: &Query) -> Option<SuggestedIndex> {
    let sort: Vec<(String, i32)> = query
        .sort
        .iter()
        .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), direction(v))))
        .collect();
    let keys = filter_fields(&query.filter, is_equality)
        .into_iter()
        .chain(sort)
        .chain(filter_fields(&query.filter, is_range))
        .fold(Vec::<(String, i32)>::new(), |acc, key| {
            if acc.iter().any(|(f, _)| *f == key.0) {
                acc
            } else {
                acc.into_iter().chain([key]).collect()
            }
        });
    (!keys.is_empty()).then_some(SuggestedIndex { keys })
}
