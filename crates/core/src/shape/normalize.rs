//! Reduces filters, sorts and projections to literal-free canonical JSON, and reads them out
//! of profiler commands.

use bson::{Bson, Document};
use serde_json::{Map, Value, json};

use super::{EXCLUDE_KEY, FILTER_KEY, INCLUDE_KEY, PLACEHOLDER, PROJECTION_KEY, SORT_KEY, Shape};
use crate::explain::suggest::direction;
use crate::store::Query;

const LOGICAL_OPS: [&str; 3] = ["$and", "$or", "$nor"];
const LIST_OPS: [&str; 3] = ["$in", "$nin", "$all"];
const ID_FIELD: &str = "_id";

fn placeholder() -> Value {
    Value::String(PLACEHOLDER.to_string())
}

fn sorted_by_text(items: Vec<Value>) -> Vec<Value> {
    let mut keyed: Vec<(String, Value)> = items.into_iter().map(|v| (v.to_string(), v)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.into_iter().map(|(_, v)| v).collect()
}

fn is_operator_doc(doc: &Document) -> bool {
    !doc.is_empty() && doc.keys().all(|k| k.starts_with('$'))
}

fn has_logical_op(doc: &Document) -> bool {
    doc.keys().any(|k| LOGICAL_OPS.contains(&k.as_str()))
}

fn sorted_entries(doc: &Document) -> Vec<(&String, &Bson)> {
    let mut entries: Vec<(&String, &Bson)> = doc.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
}

/// A query document: keys are field paths or operators and are kept, sorted.
fn query_doc(doc: &Document) -> Map<String, Value> {
    sorted_entries(doc)
        .into_iter()
        .map(|(k, v)| (k.clone(), query_entry(k, v)))
        .collect()
}

fn query_entry(key: &str, value: &Bson) -> Value {
    match value {
        Bson::Array(items) if LOGICAL_OPS.contains(&key) => Value::Array(sorted_by_text(
            items
                .iter()
                .map(|c| c.as_document().map_or_else(placeholder, query_value))
                .collect(),
        )),
        _ if key.starts_with('$') => placeholder(),
        other => field_value(other),
    }
}

fn query_value(doc: &Document) -> Value {
    Value::Object(query_doc(doc))
}

/// A field's value: an operator document keeps its operators; anything else is a literal.
fn field_value(value: &Bson) -> Value {
    match value {
        Bson::Document(doc) if is_operator_doc(doc) => Value::Object(operator_doc(doc)),
        _ => placeholder(),
    }
}

fn operator_doc(doc: &Document) -> Map<String, Value> {
    sorted_entries(doc)
        .into_iter()
        .map(|(op, operand)| (op.clone(), operand_value(op, operand)))
        .collect()
}

/// The operand of an operator: a literal, except that `$elemMatch` holds a query (or
/// operator) document and `$not` an operator document.
fn operand_value(op: &str, operand: &Bson) -> Value {
    match (op, operand) {
        (op, Bson::Array(_)) if LIST_OPS.contains(&op) => json!([PLACEHOLDER]),
        ("$elemMatch", Bson::Document(doc)) if has_logical_op(doc) || !is_operator_doc(doc) => {
            query_value(doc)
        }
        ("$elemMatch" | "$not", Bson::Document(doc)) if is_operator_doc(doc) => {
            Value::Object(operator_doc(doc))
        }
        _ => placeholder(),
    }
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
    canonical.insert(FILTER_KEY.into(), query_value(filter));
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

/// The verb to dispatch on: the profiler's `command` op names it in the first key of the
/// command document; any other op is the verb itself.
fn verb<'a>(op: &'a str, command: &'a Document) -> Option<&'a str> {
    if op == "command" {
        command.keys().next().map(String::as_str)
    } else {
        Some(op)
    }
}

/// The shape of a profiler entry. `op` is the raw `op` field of the `system.profile` entry and
/// `command` its `command` document: `query`, `update` and `remove` name the verb directly,
/// while `aggregate`, `count`, `distinct`, `findAndModify` and (on modern servers) `find`
/// arrive as `op: "command"` with the verb as the first key of `command`. `None` for ops
/// without a filter (`getmore`, `insert`, other commands) or that are unsupported.
pub fn from_profile_command(op: &str, command: &Document) -> Option<Shape> {
    let empty = Document::new();
    match verb(op, command)? {
        "query" | "find" => Some(build(
            doc(command, "filter").unwrap_or(&empty),
            doc(command, "sort"),
            doc(command, "projection"),
        )),
        "count" | "distinct" => Some(build(doc(command, "query").unwrap_or(&empty), None, None)),
        "aggregate" => aggregate_shape(command),
        "update" => statement_filter(command, "updates").map(|q| build(q, None, None)),
        "remove" | "delete" => statement_filter(command, "deletes").map(|q| build(q, None, None)),
        "findAndModify" | "findandmodify" => Some(build(
            doc(command, "query").unwrap_or(&empty),
            doc(command, "sort"),
            None,
        )),
        _ => None,
    }
}

fn to_bson(value: &Value) -> Bson {
    match value {
        Value::Object(map) => Bson::Document(to_document(map)),
        Value::Array(items) => Bson::Array(items.iter().map(to_bson).collect()),
        Value::String(s) => Bson::String(s.clone()),
        Value::Number(n) => n
            .as_i64()
            .map_or_else(|| Bson::Double(n.as_f64().unwrap_or_default()), Bson::Int64),
        Value::Bool(b) => Bson::Boolean(*b),
        Value::Null => Bson::Null,
    }
}

pub(super) fn to_document(map: &Map<String, Value>) -> Document {
    map.iter().map(|(k, v)| (k.clone(), to_bson(v))).collect()
}

/// A filter is canonical when normalising it changes nothing: no literal, no literal
/// document keys, sorted keys, `["?"]` lists. Compared as text, as a map compares unordered.
fn canonical_filter(filter: &Map<String, Value>) -> bool {
    let again = query_value(&to_document(filter));
    let (again, given) = (again.to_string(), Value::Object(filter.clone()).to_string());
    again == given
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
    if !canonical_filter(filter.as_object().ok_or("filter is not an object")?) {
        return Err("filter is not in canonical, value-free form".into());
    }
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
