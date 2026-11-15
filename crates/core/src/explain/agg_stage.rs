//! The pieces of an aggregate explain document: one pipeline stage, and the input plan under a
//! `$cursor` or a top-level `queryPlanner`. Every read is tolerant: a missing or oddly typed
//! field is `None`, never a panic and never an indexed access.

use bson::{Bson, Document};

use super::agg_input::{
    collscans_unfiltered, has_nested_loop_lookup, input_returned, input_side_sort,
};
use super::agg_model::{AggHint, AggStage, InputPlan};
use super::classify::{ACCESS_STAGES, is_eq_lookup, is_inefficient, walk};

/// Most field names a stage detail lists.
const DETAIL_KEYS: usize = 8;

/// A non-negative number in any numeric width.
pub(super) fn bson_count(value: &Bson) -> Option<u64> {
    match value {
        Bson::Int32(n) => u64::try_from(*n).ok(),
        Bson::Int64(n) => u64::try_from(*n).ok(),
        Bson::Double(n) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
        _ => None,
    }
}

/// `doc[key]` as a non-negative count; anything else is `None`.
pub(super) fn opt_count(doc: &Document, key: &str) -> Option<u64> {
    doc.get(key).and_then(bson_count)
}

/// The operator of a stage document: its first `$`-prefixed key.
pub(super) fn stage_operator(stage: &Document) -> Option<&str> {
    stage
        .keys()
        .map(String::as_str)
        .find(|key| key.starts_with('$'))
}

/// Whether a stage is the leading cursor that feeds the pipeline rather than a pipeline stage.
pub(super) fn is_input_marker(name: &str) -> bool {
    matches!(name, "$cursor" | "$geoNearCursor")
}

/// A stage and the hints it raises.
pub(super) struct ParsedStage {
    pub stage: AggStage,
    pub hints: Vec<AggHint>,
}

/// The input of a pipeline with the facts the plan summary needs.
pub(super) struct ParsedInput {
    pub plan: InputPlan,
    pub hints: Vec<AggHint>,
    pub has_stats: bool,
    pub millis: Option<u64>,
}

fn keys_text<'a>(keys: impl Iterator<Item = &'a String>) -> Option<String> {
    let joined = keys
        .take(DETAIL_KEYS)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    (!joined.is_empty()).then_some(joined)
}

fn doc_keys(body: Option<&Bson>) -> Option<String> {
    body.and_then(Bson::as_document)
        .and_then(|d| keys_text(d.keys()))
}

fn facet_names(body: Option<&Bson>) -> Option<String> {
    match body {
        Some(Bson::Array(items)) => {
            let tags: Vec<String> = items
                .iter()
                .filter_map(Bson::as_document)
                .filter_map(|d| d.get_str("tag").ok())
                .map(str::to_owned)
                .collect();
            keys_text(tags.iter())
        }
        other => doc_keys(other),
    }
}

fn str_or_field(body: Option<&Bson>, field: &str) -> Option<String> {
    match body {
        Some(Bson::String(s)) => Some(s.clone()),
        Some(Bson::Document(d)) => d.get_str(field).ok().map(str::to_owned),
        _ => None,
    }
}

/// A short description of a stage: field names and the joined collection, never a value.
fn stage_detail(name: &str, body: Option<&Bson>) -> Option<String> {
    match name {
        "$sort" => body
            .and_then(Bson::as_document)
            .and_then(|d| d.get_document("sortKey").ok())
            .and_then(|key| keys_text(key.keys())),
        "$group" => body
            .and_then(Bson::as_document)
            .and_then(|d| keys_text(d.keys().filter(|k| k.as_str() != "_id"))),
        "$match" | "$project" | "$addFields" | "$set" | "$unset" => doc_keys(body),
        "$facet" => facet_names(body),
        "$lookup" => str_or_field(body, "from"),
        "$unwind" => str_or_field(body, "path"),
        "$unionWith" => str_or_field(body, "coll"),
        "$count" => str_or_field(body, ""),
        "$limit" | "$skip" => body.and_then(bson_count).map(|n| n.to_string()),
        _ => None,
    }
}

fn stage_hints(name: &str, stage: &Document) -> Vec<AggHint> {
    let spilled = matches!(stage.get("usedDisk"), Some(Bson::Boolean(true)));
    let scanned_joined = opt_count(stage, "collectionScans").is_some_and(|n| n > 0);
    let mut hints = Vec::new();
    if spilled {
        hints.push(AggHint::DiskSort);
    } else if name == "$sort" {
        hints.push(AggHint::InMemorySort);
    }
    if name == "$lookup" && scanned_joined {
        hints.push(AggHint::LookupCollectionScan);
    }
    hints
}

/// Reads one pipeline stage document, with its operator. `None` for a document with no `$`
/// operator.
pub(super) fn parse_stage(stage: &Document) -> Option<(&str, ParsedStage)> {
    let name = stage_operator(stage)?;
    let parsed = ParsedStage {
        stage: AggStage {
            name: name.to_owned(),
            detail: stage_detail(name, stage.get(name)),
            keys_examined: opt_count(stage, "totalKeysExamined"),
            docs_examined: opt_count(stage, "totalDocsExamined"),
            n_returned: opt_count(stage, "nReturned"),
            millis: opt_count(stage, "executionTimeMillisEstimate"),
        },
        hints: stage_hints(name, stage),
    };
    Some((name, parsed))
}

/// The hint for an input that examined far more than it handed on. Only with execution figures.
/// A pushed-down `$lookup` adds the joined collection's reads to the totals, and they cannot be
/// told from the local side's, so a plan with one is not judged.
pub(super) fn examined_hint(input: &InputPlan) -> Option<AggHint> {
    if input.plan_stages.iter().any(|s| is_eq_lookup(s)) {
        return None;
    }
    let returned = input.n_returned?;
    input.keys_examined.or(input.docs_examined)?;
    is_inefficient(
        input.keys_examined.unwrap_or(0),
        input.docs_examined.unwrap_or(0),
        returned,
    )
    .then_some(AggHint::ExaminedMuchMoreThanReturned)
}

/// Reads the winning plan of `planner` (a `queryPlanner` document, or a shard entry that holds a
/// `winningPlan`) with the counters of `stats`. `None` when there is no recognisable plan.
pub(super) fn parse_input(planner: &Document, stats: Option<&Document>) -> Option<ParsedInput> {
    let winning = planner.get_document("winningPlan").ok()?;
    let (mut nodes, mut index) = (Vec::new(), None);
    walk(winning, &mut nodes, &mut index);
    let stage_kind = nodes
        .iter()
        .copied()
        .find(|s| ACCESS_STAGES.contains(s))
        .or_else(|| nodes.first().copied())?
        .to_owned();
    let mut hints = Vec::new();
    // A scan with no filter reads everything because the pipeline asks for everything: no index
    // would change that, so it is no finding (the call `classify` makes for a find, too).
    let sort_finding = input_side_sort(winning);
    let index_could_sort = sort_finding == Some(AggHint::InMemorySort);
    if nodes.contains(&"COLLSCAN") && (index_could_sort || !collscans_unfiltered(winning)) {
        hints.push(AggHint::CollectionScan);
    }
    hints.extend(sort_finding);
    if has_nested_loop_lookup(winning) {
        hints.push(AggHint::LookupCollectionScan);
    }
    let figure = |key: &str| stats.and_then(|s| opt_count(s, key));
    Some(ParsedInput {
        plan: InputPlan {
            stage_kind,
            index,
            plan_stages: nodes.iter().map(|s| (*s).to_owned()).collect(),
            keys_examined: figure("totalKeysExamined"),
            docs_examined: figure("totalDocsExamined"),
            n_returned: input_returned(&nodes, stats),
        },
        hints,
        has_stats: stats.is_some(),
        millis: figure("executionTimeMillis"),
    })
}
