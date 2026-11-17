//! The suggested index of an aggregate: the equality, sort, range rule applied to the leading
//! `$match` and the `$sort` right after it, and the command that would create it.

use bson::{Bson, Document};

use super::suggest::suggest_from;
use super::verdict::{SuggestedIndex, js_string};
use crate::pipeline_options::CollationOption;
use crate::store::Namespace;

/// The equality-sort-range index for the leading `$match` and a `$sort` directly after it (a
/// leading `$sort` alone counts as an empty match). `None` for any other first stage, a `$match`
/// with `$text`, or no field to index.
pub fn suggest_for_pipeline(stages: &[Document]) -> Option<SuggestedIndex> {
    let first = stages.first()?;
    if let Some(Bson::Document(filter)) = first.get("$match") {
        if filter.contains_key("$text") {
            return None;
        }
        let sort = stages.get(1).and_then(sort_of);
        return suggest_from(filter, sort.as_ref());
    }
    let sort = sort_of(first)?;
    suggest_from(&Document::new(), Some(&sort))
}

/// The field list of a `$sort` stage; `None` for another stage or for a sort that is not a plain
/// field list (`$natural`, a `$meta` score), which no index can serve.
fn sort_of(stage: &Document) -> Option<Document> {
    let Some(Bson::Document(sort)) = stage.get("$sort") else {
        return None;
    };
    let plain = sort.iter().all(|(field, direction)| {
        !field.starts_with('$')
            && matches!(direction, Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_))
    });
    plain.then(|| sort.clone())
}

/// `index.command(ns)` exactly without a collation; with one, the same command with
/// `{ collation: { .. } }` as its options (an index serves a query's string comparisons only with
/// the collation the query uses). Every string goes through `js_string`.
pub fn create_index_command(
    index: &SuggestedIndex,
    ns: &Namespace,
    collation: Option<&CollationOption>,
) -> String {
    let Some(collation) = collation else {
        return index.command(ns);
    };
    let keys = index
        .keys
        .iter()
        .map(|(field, dir)| format!("{}: {dir}", js_string(field)))
        .collect::<Vec<_>>()
        .join(", ");
    let fields = collation
        .to_document()
        .iter()
        .map(|(name, value)| format!("{name}: {}", js_value(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "db.getSiblingDB({}).getCollection({}).createIndex({{ {keys} }}, {{ collation: {{ {fields} }} }})",
        js_string(&ns.db),
        js_string(&ns.collection)
    )
}

/// A collation field value as JavaScript: its strings escaped, its numbers and flags bare.
fn js_value(value: &Bson) -> String {
    match value {
        Bson::String(text) => js_string(text),
        Bson::Int32(n) => n.to_string(),
        Bson::Boolean(flag) => flag.to_string(),
        // `CollationOption::to_document` writes nothing else.
        _ => "null".to_string(),
    }
}

#[cfg(test)]
#[path = "agg_suggest_tests.rs"]
mod tests;
