//! Questions about the plan tree of an aggregate input that decide its hints: whether a scan has
//! a filter, whether a `$lookup` joins by nested loop, which stage feeds a reducing stage and
//! whether a later `$sort` can still use an index. Every read is by key and tolerant.

use bson::{Bson, Document};

use super::agg_stage::opt_count;
use super::classify::is_eq_lookup;

/// Plan stages that hand on fewer documents than they read: their `nReturned` counts output
/// groups, not what the scan passed on.
const REDUCING_STAGES: [&str; 2] = ["GROUP", "COUNT"];

/// Plan stages after which a `$sort` runs on documents no index describes any more.
const RESHAPING_STAGES: [&str; 5] = ["GROUP", "COUNT", "UNWIND", "EQ_LOOKUP", "EQ_LOOKUP_UNWIND"];

/// Pipeline stages that change the shape of the documents so that no index can order them.
const RESHAPING_OPERATORS: [&str; 20] = [
    "$group",
    "$unwind",
    "$addFields",
    "$set",
    "$lookup",
    "$facet",
    "$bucket",
    "$bucketAuto",
    "$count",
    "$sortByCount",
    "$unionWith",
    "$graphLookup",
    "$replaceRoot",
    "$replaceWith",
    "$densify",
    "$fill",
    "$setWindowFields",
    "$geoNear",
    "$mergeCursors",
    "$redact",
];

/// Whether a pipeline stage (`name` with its `body`) reshapes documents. A `$project` only does
/// so when it computes a field, i.e. a value other than a plain 0/1 flag.
pub(super) fn reshapes_documents(name: &str, body: Option<&Bson>) -> bool {
    if name == "$project" {
        return body
            .and_then(Bson::as_document)
            .is_some_and(|spec| spec.values().any(|v| !is_flag(v)));
    }
    RESHAPING_OPERATORS.contains(&name)
}

fn is_flag(value: &Bson) -> bool {
    match value {
        Bson::Boolean(_) => true,
        Bson::Int32(n) => matches!(n, 0 | 1),
        Bson::Int64(n) => matches!(n, 0 | 1),
        Bson::Double(n) => *n == 0.0 || *n == 1.0,
        _ => false,
    }
}

/// Whether the plan stages of the input already reshape the documents, so a `$sort` after the
/// input cannot be served by an index.
pub(super) fn input_reshapes(plan_stages: &[&str]) -> bool {
    plan_stages.iter().any(|s| RESHAPING_STAGES.contains(s))
}

fn children(node: &Document) -> Vec<&Document> {
    let mut found: Vec<&Document> = ["queryPlan", "inputStage"]
        .iter()
        .filter_map(|key| node.get_document(key).ok())
        .collect();
    if let Ok(list) = node.get_array("inputStages") {
        // A pushed-down `$lookup` lists the local side first, the joined collection second.
        let wanted = match node.get_str("stage") {
            Ok(stage) if is_eq_lookup(stage) => 1,
            _ => usize::MAX,
        };
        found.extend(list.iter().filter_map(Bson::as_document).take(wanted));
    }
    found
}

/// Whether a `$lookup` pushed into the plan joins by nested loop: it reads the joined collection
/// whole for each local document. Indexed, hash and missing-collection joins are not that.
pub(super) fn has_nested_loop_lookup(node: &Document) -> bool {
    let here = node.get_str("stage").is_ok_and(is_eq_lookup)
        && node.get_str("strategy") == Ok("NestedLoopJoin");
    here || ["queryPlan", "inputStage"]
        .iter()
        .filter_map(|key| node.get_document(key).ok())
        .any(has_nested_loop_lookup)
        || node.get_array("inputStages").is_ok_and(|list| {
            list.iter()
                .filter_map(Bson::as_document)
                .any(has_nested_loop_lookup)
        })
}

/// Whether every collection scan of the plan has no filter (a missing or empty one): the scan
/// reads the collection because the pipeline asked for all of it, not for lack of an index.
pub(super) fn collscans_unfiltered(node: &Document) -> bool {
    let own = match node.get_str("stage") {
        Ok("COLLSCAN") => node.get_document("filter").map_or(true, Document::is_empty),
        _ => true,
    };
    own && children(node).into_iter().all(collscans_unfiltered)
}

/// How many documents the stage feeding a reducing stage returned, found in `execution` (an
/// `executionStages` tree). `None` when there is no reducing stage or its input has no figure.
fn feeding_returned(execution: &Document) -> Option<u64> {
    let reducing = execution
        .get_str("stage")
        .is_ok_and(|s| REDUCING_STAGES.contains(&s));
    if reducing {
        let input = ["inputStage", "queryPlan"]
            .iter()
            .find_map(|key| execution.get_document(key).ok())?;
        return opt_count(input, "nReturned");
    }
    children(execution).into_iter().find_map(feeding_returned)
}

/// What the input handed on. Without a reducing stage that is the `nReturned` of `stats`. With
/// one (a `$group` folded into the plan) that count is the number of output groups, so the
/// figure comes from the stage that fed the reducer, and is `None` when it cannot be found.
pub(super) fn input_returned(plan_stages: &[&str], stats: Option<&Document>) -> Option<u64> {
    let stats = stats?;
    if !plan_stages.iter().any(|s| REDUCING_STAGES.contains(s)) {
        return opt_count(stats, "nReturned");
    }
    feeding_returned(stats.get_document("executionStages").ok()?)
}
