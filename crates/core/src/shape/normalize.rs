//! Reduces filters, sorts and projections to literal-free canonical JSON, and reads them out
//! of profiler commands.

use bson::{Bson, Document};
use serde_json::{Map, Value, json};

use super::{EXCLUDE_KEY, FILTER_KEY, INCLUDE_KEY, PLACEHOLDER, PROJECTION_KEY, SORT_KEY, Shape};
use crate::explain::suggest::direction;
use crate::store::Query;

const LOGICAL_OPS: [&str; 3] = ["$and", "$or", "$nor"];
const LIST_OPS: [&str; 2] = ["$in", "$nin"];
const ID_FIELD: &str = "_id";

fn placeholder() -> Value {
    Value::String(PLACEHOLDER.to_string())
}

fn dedup(items: Vec<Value>) -> Vec<Value> {
    items.into_iter().fold(Vec::new(), |acc, item| {
        if acc.contains(&item) {
            acc
        } else {
            acc.into_iter().chain([item]).collect()
        }
    })
}

fn sorted_by_text(items: Vec<Value>) -> Vec<Value> {
    let mut keyed: Vec<(String, Value)> = items.into_iter().map(|v| (v.to_string(), v)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.into_iter().map(|(_, v)| v).collect()
}

fn value_for(key: &str, value: &Bson) -> Value {
    match value {
        Bson::Array(_) if LIST_OPS.contains(&key) => json!([PLACEHOLDER]),
        Bson::Array(items) if LOGICAL_OPS.contains(&key) => {
            Value::Array(sorted_by_text(items.iter().map(literal_free).collect()))
        }
        other => literal_free(other),
    }
}

/// Keys and operators kept (keys sorted), every scalar masked, scalar arrays collapsed.
fn literal_free(value: &Bson) -> Value {
    match value {
        Bson::Document(doc) => Value::Object(object_of(doc)),
        Bson::Array(items) => Value::Array(dedup(items.iter().map(literal_free).collect())),
        _ => placeholder(),
    }
}

fn object_of(doc: &Document) -> Map<String, Value> {
    let mut entries: Vec<(&String, &Bson)> = doc.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
        .into_iter()
        .map(|(k, v)| (k.clone(), value_for(k, v)))
        .collect()
}

fn sort_of(sort: &Document) -> Map<String, Value> {
    sort.iter()
        .map(|(k, v)| (k.clone(), json!(direction(v))))
        .collect()
}

fn is_zero(value: &Bson) -> bool {
    match value {
        Bson::Int32(n) => *n == 0,
        Bson::Int64(n) => *n == 0,
        Bson::Double(n) => *n == 0.0,
        Bson::Boolean(b) => !*b,
        _ => false,
    }
}

/// Field names only, sorted; an exclusion when every field but `_id` is switched off.
fn projection_of(projection: &Document) -> Map<String, Value> {
    let exclusion = projection
        .iter()
        .filter(|(k, _)| k.as_str() != ID_FIELD)
        .all(|(_, v)| is_zero(v))
        && !projection.is_empty();
    let mut names: Vec<&String> = projection.keys().collect();
    names.sort();
    let key = if exclusion { EXCLUDE_KEY } else { INCLUDE_KEY };
    Map::from_iter([(key.to_string(), json!(names))])
}

fn build(filter: &Document, sort: Option<&Document>, projection: Option<&Document>) -> Shape {
    let mut canonical = Map::new();
    canonical.insert(FILTER_KEY.into(), Value::Object(object_of(filter)));
    if let Some(sort) = sort.filter(|s| !s.is_empty()) {
        canonical.insert(SORT_KEY.into(), Value::Object(sort_of(sort)));
    }
    if let Some(p) = projection.filter(|p| !p.is_empty()) {
        canonical.insert(PROJECTION_KEY.into(), Value::Object(projection_of(p)));
    }
    Shape::new(canonical)
}

/// The shape of a find request; `skip` and `limit` are not part of it.
pub fn from_query(q: &Query) -> Shape {
    build(&q.filter, q.sort.as_ref(), q.projection.as_ref())
}

fn doc<'a>(command: &'a Document, key: &str) -> Option<&'a Document> {
    command.get_document(key).ok()
}

/// `q` of the first statement under `list` (`updates`, `deletes`), or the command's own `q`
/// (the profiler shows an update or delete statement as the command itself).
fn statement_filter<'a>(command: &'a Document, list: &str) -> Option<&'a Document> {
    command
        .get_array(list)
        .ok()
        .and_then(|statements| statements.first())
        .and_then(Bson::as_document)
        .and_then(|statement| doc(statement, "q"))
        .or_else(|| doc(command, "q"))
}

fn aggregate_shape(command: &Document) -> Option<Shape> {
    let stages = command.get_array("pipeline").ok()?;
    let matched = stages
        .first()
        .and_then(Bson::as_document)
        .and_then(|stage| doc(stage, "$match"))?;
    let sort = stages
        .get(1)
        .and_then(Bson::as_document)
        .and_then(|stage| doc(stage, "$sort"));
    Some(build(matched, sort, None))
}

/// The shape of a profiler entry's `command`, per op. `None` for ops without a filter
/// (`getMore`, `insert`, commands) or that are unsupported.
pub fn from_profile_command(op: &str, command: &Document) -> Option<Shape> {
    let empty = Document::new();
    match op {
        "query" | "find" => Some(build(
            doc(command, "filter").unwrap_or(&empty),
            doc(command, "sort"),
            doc(command, "projection"),
        )),
        "count" | "distinct" => Some(build(doc(command, "query").unwrap_or(&empty), None, None)),
        "aggregate" => aggregate_shape(command),
        "update" => statement_filter(command, "updates").map(|q| build(q, None, None)),
        "remove" | "delete" => statement_filter(command, "deletes").map(|q| build(q, None, None)),
        "findAndModify" => Some(build(
            doc(command, "query").unwrap_or(&empty),
            doc(command, "sort"),
            None,
        )),
        _ => None,
    }
}

fn value_free(value: &Value) -> Result<(), String> {
    match value {
        Value::Object(map) => map.values().try_for_each(value_free),
        Value::Array(items) => items.iter().try_for_each(value_free),
        Value::String(s) if s == PLACEHOLDER => Ok(()),
        _ => Err("a literal value".to_string()),
    }
}

fn names(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().all(Value::is_string))
}

fn valid_sort(value: &Value) -> bool {
    value.as_object().is_some_and(|map| {
        map.values()
            .all(|v| matches!(v.as_i64(), Some(1) | Some(-1)))
    })
}

fn valid_projection(value: &Value) -> bool {
    value.as_object().is_some_and(|map| {
        map.len() == 1
            && [INCLUDE_KEY, EXCLUDE_KEY]
                .iter()
                .any(|k| map.get(*k).is_some_and(names))
    })
}

/// Checks that `canonical` has the layout and the value-free content `build` produces.
pub(super) fn validate(canonical: &Map<String, Value>) -> Result<(), String> {
    let filter = canonical.get(FILTER_KEY).ok_or("no filter")?;
    if !filter.is_object() {
        return Err("filter is not an object".into());
    }
    value_free(filter)?;
    if canonical
        .keys()
        .any(|k| ![FILTER_KEY, SORT_KEY, PROJECTION_KEY].contains(&k.as_str()))
    {
        return Err("unknown section".into());
    }
    if canonical.get(SORT_KEY).is_some_and(|s| !valid_sort(s)) {
        return Err("bad sort".into());
    }
    if canonical
        .get(PROJECTION_KEY)
        .is_some_and(|p| !valid_projection(p))
    {
        return Err("bad projection".into());
    }
    Ok(())
}
