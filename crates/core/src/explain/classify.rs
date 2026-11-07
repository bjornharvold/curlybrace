//! Judges an explain document: was the query served well by an index?

use bson::{Bson, Document};

use super::suggest::suggest;
use super::verdict::{Reason, Verdict, VerdictKind};
use crate::error::{CoreError, Result};
use crate::store::Query;

/// Examined documents/keys below this are never called inefficient.
const INEFFICIENT_MIN_EXAMINED: u64 = 100;
/// Examined must be at least this many times the returned count.
const INEFFICIENT_RATIO: u64 = 10;

fn count(doc: Option<&Document>, key: &str) -> u64 {
    match doc.and_then(|d| d.get(key)) {
        Some(Bson::Int32(n)) => u64::try_from(*n).unwrap_or(0),
        Some(Bson::Int64(n)) => u64::try_from(*n).unwrap_or(0),
        Some(Bson::Double(n)) if *n >= 0.0 => *n as u64,
        _ => 0,
    }
}

/// Every stage in the plan tree (pre-order), with the first index name found.
fn walk<'a>(node: &'a Document, stages: &mut Vec<&'a str>, index: &mut Option<String>) {
    if let Ok(stage) = node.get_str("stage") {
        stages.push(stage);
        if index.is_none() {
            *index = match stage {
                "IXSCAN" => node.get_str("indexName").ok().map(str::to_owned),
                "IDHACK" => Some("_id_".to_owned()),
                _ => None,
            };
        }
    }
    for key in ["inputStage", "queryPlan"] {
        if let Ok(child) = node.get_document(key) {
            walk(child, stages, index);
        }
    }
    if let Ok(children) = node.get_array("inputStages") {
        children
            .iter()
            .filter_map(Bson::as_document)
            .for_each(|c| walk(c, stages, index));
    }
}

fn is_inefficient(keys: u64, docs: u64, returned: u64) -> bool {
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
}

impl Plan<'_> {
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
        } else if self.has("IXSCAN") && !self.plan_only && self.inefficient {
            VerdictKind::InefficientIndex
        } else if self.has("PROJECTION_COVERED") || (self.has("IXSCAN") && !self.has("FETCH")) {
            VerdictKind::CoveredByIndex
        } else {
            VerdictKind::IndexUsed
        }
    }
}

fn reasons(kind: VerdictKind, keys: u64, docs: u64, returned: u64) -> Vec<Reason> {
    match kind {
        VerdictKind::CollectionScan => vec![Reason::new("collscan")],
        VerdictKind::SortNotCovered => vec![Reason::new("sort_blocking")],
        VerdictKind::InefficientIndex => vec![
            Reason::new("inefficient")
                .with_param("examined", keys.max(docs))
                .with_param("returned", returned),
        ],
        VerdictKind::NoFilter => vec![Reason::new("no_filter")],
        VerdictKind::EmptyCollection => vec![Reason::new("empty_collection")],
        VerdictKind::IndexUsed | VerdictKind::CoveredByIndex => Vec::new(),
    }
}

fn wants_suggestion(kind: VerdictKind) -> bool {
    matches!(
        kind,
        VerdictKind::CollectionScan | VerdictKind::SortNotCovered | VerdictKind::InefficientIndex
    )
}

/// Classifies an explain document produced for `query`. Errors with
/// `CoreError::Database` when the document has no recognisable winning plan.
pub fn classify(explain: &Document, query: &Query) -> Result<Verdict> {
    let winning = explain
        .get_document("queryPlanner")
        .and_then(|p| p.get_document("winningPlan"))
        .map_err(|_| CoreError::Database("explain output has no winning plan".into()))?;
    let (mut stages, mut index_name) = (Vec::new(), None);
    walk(winning, &mut stages, &mut index_name);
    if stages.is_empty() {
        return Err(CoreError::Database(
            "explain winning plan has no stages".into(),
        ));
    }
    let stats = explain.get_document("executionStats").ok();
    let (keys, docs, returned) = (
        count(stats, "totalKeysExamined"),
        count(stats, "totalDocsExamined"),
        count(stats, "nReturned"),
    );
    let scanned_nothing = stats.is_some() && stages.contains(&"COLLSCAN") && docs == 0;
    let plan = Plan {
        stages,
        has_sort_key: query.sort.as_ref().is_some_and(|s| !s.is_empty()),
        filter_empty: query.filter.is_empty(),
        plan_only: stats.is_none(),
        inefficient: is_inefficient(keys, docs, returned),
        scanned_nothing,
    };
    let kind = plan.kind();
    Ok(Verdict {
        kind,
        index_name,
        keys_examined: keys,
        docs_examined: docs,
        returned,
        plan_only: plan.plan_only,
        reasons: reasons(kind, keys, docs, returned),
        suggestion: wants_suggestion(kind).then(|| suggest(query)).flatten(),
    })
}
