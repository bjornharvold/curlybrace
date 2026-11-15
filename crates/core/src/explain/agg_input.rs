//! Questions about the plan tree of an aggregate input that decide its hints: whether a scan has
//! a filter, whether a `$lookup` joins by nested loop, which stage feeds a reducing stage and
//! whether a later `$sort` can still use an index. Every read is by key and tolerant.

use bson::{Bson, Document};

use super::agg_model::AggHint;
use super::agg_stage::opt_count;
use super::classify::is_eq_lookup;

/// Whether a plan stage hands on fewer documents than it reads: its `nReturned` counts output
/// groups, not what the scan passed on. The plan tree names it `GROUP`, the slot-based execution
/// tree `group`.
fn is_reducing(stage: &str) -> bool {
    stage.eq_ignore_ascii_case("group") || stage.eq_ignore_ascii_case("count")
}

/// Plan stages after which a `$sort` runs on documents no index describes any more. A `LIMIT` is
/// one: the server folds a leading `$limit` into the `$cursor` plan, and the `$sort` after it
/// stays a pipeline stage that sorts the few documents that are left. A `SKIP` is not: it hides
/// the index from the sort the same way, but what the sort then orders is not small.
const RESHAPING_STAGES: [&str; 6] = [
    "GROUP",
    "COUNT",
    "UNWIND",
    "EQ_LOOKUP",
    "EQ_LOOKUP_UNWIND",
    "LIMIT",
];

/// The plan stage a `$skip` folds into.
const SKIP_STAGE: &str = "SKIP";

/// What the stages before a `$sort` leave of its chance to be served by an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SortOrder {
    /// Nothing before the sort changes the documents or their count: an index could serve it.
    Indexable,
    /// A `$skip` came first: no index can serve the sort, yet it orders every document the skip
    /// left, so the cost is real.
    AfterSkip,
    /// Reshaped or limited documents: the sort is not a finding.
    Unindexable,
}

impl SortOrder {
    /// The state after the plan stages folded into the `$cursor`.
    pub(super) fn after_plan(plan_stages: &[&str]) -> Self {
        if plan_stages.iter().any(|s| RESHAPING_STAGES.contains(s)) {
            Self::Unindexable
        } else if plan_stages.contains(&SKIP_STAGE) {
            Self::AfterSkip
        } else {
            Self::Indexable
        }
    }

    /// The state after the pipeline stage `name` (with its `body`).
    pub(super) fn after_stage(self, name: &str, body: Option<&Bson>) -> Self {
        match (name, self) {
            ("$skip", Self::Indexable) => Self::AfterSkip,
            (_, Self::Unindexable) => Self::Unindexable,
            _ if reshapes_documents(name, body) => Self::Unindexable,
            _ => self,
        }
    }

    /// The hint a blocking sort raises here: none, the skip variant, or the plain one.
    pub(super) fn sort_hint(self) -> Option<AggHint> {
        match self {
            Self::Indexable => Some(AggHint::InMemorySort),
            Self::AfterSkip => Some(AggHint::InMemorySortAfterSkip),
            Self::Unindexable => None,
        }
    }

    /// `hints` with an in-memory sort finding turned into what this state says of it.
    pub(super) fn adjust(self, hints: &mut Vec<AggHint>) {
        if let Some(position) = hints.iter().position(|h| *h == AggHint::InMemorySort) {
            match self.sort_hint() {
                Some(hint) => hints[position] = hint,
                None => {
                    hints.remove(position);
                }
            }
        }
    }
}

/// Pipeline stages that change the shape of the documents so that no index can order them.
const RESHAPING_OPERATORS: [&str; 31] = [
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
    // A `$sort` after a `$limit` sorts only the few documents that are left, which is cheap. (A
    // `$skip` is handled by `SortOrder`: that sort is a finding, without the index promise.)
    "$limit",
    // What the server prints for the stages that desugar.
    "$_internalSetWindowFields",
    "$_internalDensify",
    "$_internalSearchMongotRemote",
    "$_internalSearchIdLookup",
    "$vectorSearch",
    "$sample",
    "$search",
    "$searchMeta",
    "$listSessions",
    "$documents",
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

/// A plain inclusion/exclusion flag. The server prints a nested inclusion `{"a.b": 1}` as
/// `{"a": {"b": true}}`, so a sub-document with no operator whose leaves are all flags is one.
fn is_flag(value: &Bson) -> bool {
    match value {
        Bson::Document(nested) => {
            !nested.is_empty()
                && nested
                    .iter()
                    .all(|(key, leaf)| !key.starts_with('$') && is_flag(leaf))
        }
        Bson::Boolean(_) => true,
        Bson::Int32(n) => matches!(n, 0 | 1),
        Bson::Int64(n) => matches!(n, 0 | 1),
        Bson::Double(n) => *n == 0.0 || *n == 1.0,
        _ => false,
    }
}

/// The finding for a `SORT` on the input side of the plan: one with no reshaping stage between it
/// and the scan. A `SORT` above a `GROUP`, `UNWIND` or `EQ_LOOKUP` orders reshaped documents no
/// index describes; one above a `SKIP` is the skip variant. `None` when there is no such sort.
pub(super) fn input_side_sort(node: &Document) -> Option<AggHint> {
    let kids = children(node);
    let own = (node.get_str("stage") == Ok("SORT"))
        .then(|| {
            let mut below = Vec::new();
            kids.iter().for_each(|k| collect_stages(k, &mut below));
            SortOrder::after_plan(&below).sort_hint()
        })
        .flatten();
    own.or_else(|| kids.into_iter().find_map(input_side_sort))
}

/// Every plan stage name at or under `node` that the plan walk visits.
fn collect_stages<'a>(node: &'a Document, found: &mut Vec<&'a str>) {
    found.extend(node.get_str("stage").ok());
    children(node)
        .into_iter()
        .for_each(|child| collect_stages(child, found));
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
    here || children(node).into_iter().any(has_nested_loop_lookup)
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
    if execution.get_str("stage").is_ok_and(is_reducing) {
        let input = ["inputStage", "queryPlan", "outerStage"]
            .iter()
            .find_map(|key| execution.get_document(key).ok())?;
        return opt_count(input, "nReturned");
    }
    execution_children(execution)
        .into_iter()
        .find_map(feeding_returned)
}

/// The children of a node of an `executionStages` tree. The slot-based engine links a nested
/// loop join through `outerStage` and `innerStage`.
fn execution_children(node: &Document) -> Vec<&Document> {
    let mut found: Vec<&Document> = ["queryPlan", "inputStage", "outerStage", "innerStage"]
        .iter()
        .filter_map(|key| node.get_document(key).ok())
        .collect();
    if let Ok(list) = node.get_array("inputStages") {
        found.extend(list.iter().filter_map(Bson::as_document));
    }
    found
}

/// What the input handed on. Without a reducing stage that is the `nReturned` of `stats`. With
/// one (a `$group` folded into the plan) that count is the number of output groups, so the
/// figure comes from the stage that fed the reducer, and is `None` when it cannot be found.
pub(super) fn input_returned(plan_stages: &[&str], stats: Option<&Document>) -> Option<u64> {
    let stats = stats?;
    if !plan_stages.iter().any(|s| is_reducing(s)) {
        return opt_count(stats, "nReturned");
    }
    feeding_returned(stats.get_document("executionStages").ok()?)
}
