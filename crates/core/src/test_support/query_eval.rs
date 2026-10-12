//! Filter, sort and projection evaluation for `InMemoryStore`.

use std::cmp::Ordering;

use bson::{Bson, Document};

use super::bson_order::cmp_bson;
use crate::error::{CoreError, Result};

const NULL: Bson = Bson::Null;

fn unsupported(what: &str) -> CoreError {
    CoreError::Unsupported(format!("in-memory store: {what}"))
}

/// Validates that `filter` only uses top-level equality conditions.
pub fn check_filter(filter: &Document) -> Result<()> {
    for (key, value) in filter {
        if key.starts_with('$') || key.contains('.') {
            return Err(unsupported(&format!("filter key `{key}`")));
        }
        if matches!(value, Bson::Document(d) if d.keys().any(|k| k.starts_with('$'))) {
            return Err(unsupported(&format!("operator in filter field `{key}`")));
        }
    }
    Ok(())
}

/// Equality match on top-level fields; a missing field equals `null`.
pub fn matches(doc: &Document, filter: &Document) -> bool {
    filter
        .iter()
        .all(|(k, want)| cmp_bson(doc.get(k).unwrap_or(&NULL), want) == Ordering::Equal)
}

/// Parses a sort spec into `(field, ascending)` pairs.
pub fn parse_sort(sort: &Document) -> Result<Vec<(String, bool)>> {
    sort.iter()
        .map(|(k, v)| match v {
            Bson::Int32(1) | Bson::Int64(1) => Ok((k.clone(), true)),
            Bson::Int32(-1) | Bson::Int64(-1) => Ok((k.clone(), false)),
            Bson::Double(d) if *d == 1.0 => Ok((k.clone(), true)),
            Bson::Double(d) if *d == -1.0 => Ok((k.clone(), false)),
            _ => Err(CoreError::InvalidDocument(format!("sort direction for `{k}` must be 1 or -1"))),
        })
        .collect()
}

/// Compares two documents by the parsed sort keys (stable for ties).
pub fn compare(a: &Document, b: &Document, keys: &[(String, bool)]) -> Ordering {
    for (field, ascending) in keys {
        let ord = cmp_bson(a.get(field).unwrap_or(&NULL), b.get(field).unwrap_or(&NULL));
        let ord = if *ascending { ord } else { ord.reverse() };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn flag(key: &str, v: &Bson) -> Result<bool> {
    match v {
        Bson::Boolean(b) => Ok(*b),
        Bson::Int32(n) => Ok(*n != 0),
        Bson::Int64(n) => Ok(*n != 0),
        Bson::Double(n) => Ok(*n != 0.0),
        _ => Err(unsupported(&format!("projection value for `{key}`"))),
    }
}

/// Applies a projection: inclusion (`_id` kept unless `_id: 0`) or exclusion.
pub fn project(doc: &Document, projection: &Document) -> Result<Document> {
    let mut include_id = true;
    let mut fields = Vec::new();
    for (key, value) in projection {
        if key.contains('.') {
            return Err(unsupported(&format!("projection path `{key}`")));
        }
        let on = flag(key, value)?;
        if key == "_id" {
            include_id = on;
        } else {
            fields.push((key.as_str(), on));
        }
    }
    let inclusion = fields.first().map(|(_, on)| *on);
    if fields.iter().any(|(_, on)| Some(*on) != inclusion) {
        return Err(CoreError::InvalidDocument("projection mixes inclusion and exclusion".into()));
    }
    let names: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
    Ok(doc
        .iter()
        .filter(|(k, _)| match (k.as_str(), inclusion) {
            ("_id", _) => include_id,
            (k, Some(true)) => names.contains(&k),
            (k, Some(false)) => !names.contains(&k),
            (_, None) => true,
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect())
}
