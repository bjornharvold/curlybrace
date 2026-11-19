//! Judges an explain document: was the query served well by an index?

use bson::{Bson, Document};

use super::field_order::{FieldOrderFinding, check as check_field_order};
use super::suggest::suggest;
use super::verdict::{Reason, Verdict, VerdictKind};
use crate::error::{CoreError, Result};
use crate::store::Query;

/// Examined documents/keys below this are never called inefficient.
const INEFFICIENT_MIN_EXAMINED: u64 = 100;
/// Examined must be at least this many times the returned count.
const INEFFICIENT_RATIO: u64 = 10;

/// A non-negative count read from `doc[key]` in any numeric width; anything else reads as 0.
pub(crate) fn count(doc: Option<&Document>, key: &str) -> u64 {
    match doc.and_then(|d| d.get(key)) {
        Some(Bson::Int32(n)) => u64::try_from(*n).unwrap_or(0),
        Some(Bson::Int64(n)) => u64::try_from(*n).unwrap_or(0),
        Some(Bson::Double(n)) if *n >= 0.0 => *n as u64,
        _ => 0,
    }
}

/// Whether a plan stage is a `$lookup` pushed into the plan (`EQ_LOOKUP`, or the variant that
/// also unwinds the joined array).
pub(crate) fn is_eq_lookup(stage: &str) -> bool {
    matches!(stage, "EQ_LOOKUP" | "EQ_LOOKUP_UNWIND")
}

/// Where a plan walk records the first index it finds.
#[derive(Default)]
pub(crate) struct FirstIndex<'a> {
    /// The index name.
    pub(crate) name: Option<String>,
    /// The key pattern of the index scan that found it (none for an `_id` lookup).
    pub(crate) key_pattern: Option<&'a Document>,
}

/// The stages that scan an index (and carry its `keyPattern`).
const INDEX_SCAN_STAGES: [&str; 3] = ["IXSCAN", "EXPRESS_IXSCAN", "EXPRESS_CLUSTERED_IXSCAN"];

/// Every stage in the plan tree (pre-order), with the first index found.
pub(crate) fn walk<'a>(node: &'a Document, stages: &mut Vec<&'a str>, first: &mut FirstIndex<'a>) {
    if let Ok(stage) = node.get_str("stage") {
        stages.push(stage);
        if first.name.is_none() {
            match stage {
                s if INDEX_SCAN_STAGES.contains(&s) => {
                    first.name = node.get_str("indexName").ok().map(str::to_owned);
                    first.key_pattern = node.get_document("keyPattern").ok();
                }
                "IDHACK" | "EXPRESS_IDHACK" => first.name = Some("_id_".to_owned()),
                _ => {}
            }
        }
    }
    for key in ["inputStage", "queryPlan"] {
        if let Ok(child) = node.get_document(key) {
            walk(child, stages, first);
        }
    }
    if let Ok(children) = node.get_array("inputStages") {
        // A pushed-down `$lookup` (`EQ_LOOKUP`) lists the local side first and the joined
        // collection's plan second: that one says nothing about how this collection is read.
        let wanted = match node.get_str("stage") {
            Ok(stage) if is_eq_lookup(stage) => 1,
            _ => usize::MAX,
        };
        children
            .iter()
            .filter_map(Bson::as_document)
            .take(wanted)
            .for_each(|c| walk(c, stages, first));
    }
    // mongos: the merge stage on top, each shard's own plan under `shards[].winningPlan`.
    if let Ok(shards) = node.get_array("shards") {
        shards
            .iter()
            .filter_map(Bson::as_document)
            .filter_map(|s| s.get_document("winningPlan").ok())
            .for_each(|plan| walk(plan, stages, first));
    }
}

/// Stages that read documents or index entries. A plan with none of them says nothing about
/// whether an index served the query, so it is not judged.
pub(crate) const ACCESS_STAGES: [&str; 15] = [
    "COLLSCAN",
    "IXSCAN",
    "IDHACK",
    "EOF",
    "EXPRESS_IXSCAN",
    "EXPRESS_CLUSTERED_IXSCAN",
    "EXPRESS_IDHACK",
    "COUNT_SCAN",
    "DISTINCT_SCAN",
    "TEXT_MATCH",
    "TEXT_OR",
    "GEO_NEAR_2D",
    "GEO_NEAR_2DSPHERE",
    "RECORD_STORE_FAST_COUNT",
    "VIRTUAL_SCAN",
];

fn has_access_stage(stages: &[&str]) -> bool {
    stages.iter().any(|s| ACCESS_STAGES.contains(s))
}

/// Whether examining `keys`/`docs` to return `returned` is wasteful. Shared with the profiler so
/// both judge by the same thresholds.
pub(crate) fn is_inefficient(keys: u64, docs: u64, returned: u64) -> bool {
    let examined = keys.max(docs);
    examined >= INEFFICIENT_MIN_EXAMINED && examined >= INEFFICIENT_RATIO * returned.max(1)
}

struct Plan<'a> {
    stages: Vec<&'a str>,
    has_sort_key: bool,
    filter_empty: bool,
    plan_only: bool,
    inefficient: bool,
    /// Statistics are present and a collection scan read no document: the collection is empty.
    scanned_nothing: bool,
    /// The order problem of the one index the plan scans, when it can be judged.
    field_order: Option<FieldOrderFinding>,
}

impl Plan<'_> {
    fn range_before_equality(&self) -> bool {
        matches!(
            self.field_order,
            Some(FieldOrderFinding::RangeBeforeEquality { .. })
        )
    }

    fn has(&self, stage: &str) -> bool {
        self.stages.contains(&stage)
    }

    fn kind(&self) -> VerdictKind {
        // Clustered-collection edge: 0 docs examined can also mean the `_id` bounds matched
        // nothing in a non-empty clustered collection; that reads as empty here too.
        // The server plans EOF for a collection that does not exist, but a COLLSCAN that read
        // nothing for one that exists and is empty; both are empty, whatever the filter.
        if self.has("EOF") || self.scanned_nothing {
            VerdictKind::EmptyCollection
        } else if self.filter_empty && !self.has_sort_key && self.has("COLLSCAN") {
            VerdictKind::NoFilter
        } else if self.has("SORT") {
            VerdictKind::SortNotCovered
        } else if self.has("COLLSCAN") {
            VerdictKind::CollectionScan
        } else if self.range_before_equality() {
            VerdictKind::FieldOrder
        } else if self.has("IXSCAN") && !self.plan_only && self.inefficient {
            VerdictKind::InefficientIndex
        } else if self.has("PROJECTION_COVERED") || (self.has("IXSCAN") && !self.has("FETCH")) {
            VerdictKind::CoveredByIndex
        } else {
            VerdictKind::IndexUsed
        }
    }
}

/// The plain-words reason for an order problem. Params are field and index names only.
fn field_order_reason(finding: &FieldOrderFinding, index: &str) -> Reason {
    let (key, range, field) = match finding {
        FieldOrderFinding::RangeBeforeEquality { range, field } => ("field_order", range, field),
        FieldOrderFinding::RangeBeforeSort { range, field } => ("field_order_sort", range, field),
    };
    Reason::new(key)
        .with_param("index", index)
        .with_param("range", range)
        .with_param("field", field)
}

fn inefficient_reason(keys: u64, docs: u64, returned: u64) -> Reason {
    Reason::new("inefficient")
        .with_param("examined", keys.max(docs))
        .with_param("returned", returned)
}

fn reasons(
    plan: &Plan,
    kind: VerdictKind,
    (keys, docs, returned): (u64, u64, u64),
    index: &str,
) -> Vec<Reason> {
    let order = plan
        .field_order
        .iter()
        .map(|finding| field_order_reason(finding, index));
    match kind {
        VerdictKind::CollectionScan => vec![Reason::new("collscan")],
        // A blocking sort over a full scan: the scan is the more severe news, so both show.
        VerdictKind::SortNotCovered if plan.has("COLLSCAN") => {
            vec![Reason::new("sort_blocking"), Reason::new("collscan")]
        }
        VerdictKind::SortNotCovered => [Reason::new("sort_blocking")]
            .into_iter()
            .chain(order)
            .collect(),
        VerdictKind::FieldOrder => order
            .chain(
                (!plan.plan_only && plan.inefficient)
                    .then(|| inefficient_reason(keys, docs, returned)),
            )
            .collect(),
        VerdictKind::InefficientIndex => vec![inefficient_reason(keys, docs, returned)],
        VerdictKind::NoFilter => vec![Reason::new("no_filter")],
        VerdictKind::EmptyCollection => vec![Reason::new("empty_collection")],
        VerdictKind::IndexUsed | VerdictKind::CoveredByIndex => Vec::new(),
    }
}

fn wants_suggestion(kind: VerdictKind) -> bool {
    matches!(
        kind,
        VerdictKind::CollectionScan
            | VerdictKind::SortNotCovered
            | VerdictKind::InefficientIndex
            | VerdictKind::FieldOrder
    )
}

/// The order problem of the index the plan scans: judged only for a plan with exactly one index
/// scan whose key pattern is known (several scans, as in an `$or`, are not judged).
fn judge_field_order(
    stages: &[&str],
    key_pattern: Option<&Document>,
    query: &Query,
) -> Option<FieldOrderFinding> {
    let scans = stages
        .iter()
        .filter(|s| INDEX_SCAN_STAGES.contains(s))
        .count();
    key_pattern
        .filter(|_| scans == 1)
        .and_then(|keys| check_field_order(keys, &query.filter, query.sort.as_ref()))
}

/// Classifies an explain document produced for `query`. Errors with
/// `CoreError::Database` when the document has no recognisable winning plan.
pub fn classify(explain: &Document, query: &Query) -> Result<Verdict> {
    let planner = explain.get_document("queryPlanner");
    let namespace = planner
        .as_ref()
        .ok()
        .and_then(|p| p.get_str("namespace").ok())
        .map(str::to_owned);
    let winning = planner
        .and_then(|p| p.get_document("winningPlan"))
        .map_err(|_| CoreError::Database("explain output has no winning plan".into()))?;
    let (mut stages, mut first) = (Vec::new(), FirstIndex::default());
    walk(winning, &mut stages, &mut first);
    if !has_access_stage(&stages) {
        return Err(CoreError::Database(
            "explain winning plan has no access stage".into(),
        ));
    }
    let stats = explain.get_document("executionStats").ok();
    let (keys, docs, returned) = (
        count(stats, "totalKeysExamined"),
        count(stats, "totalDocsExamined"),
        count(stats, "nReturned"),
    );
    let scanned_nothing = stats.is_some() && stages.contains(&"COLLSCAN") && docs == 0;
    let field_order = judge_field_order(&stages, first.key_pattern, query);
    let plan = Plan {
        stages,
        has_sort_key: query.sort.as_ref().is_some_and(|s| !s.is_empty()),
        filter_empty: query.filter.is_empty(),
        plan_only: stats.is_none(),
        inefficient: is_inefficient(keys, docs, returned),
        scanned_nothing,
        field_order,
    };
    let kind = plan.kind();
    let index_name = first.name;
    Ok(Verdict {
        kind,
        reasons: reasons(
            &plan,
            kind,
            (keys, docs, returned),
            index_name.as_deref().unwrap_or_default(),
        ),
        index_name,
        keys_examined: keys,
        docs_examined: docs,
        returned,
        plan_only: plan.plan_only,
        suggestion: wants_suggestion(kind).then(|| suggest(query)).flatten(),
        namespace,
    })
}
