use super::*;
use curlybrace_core::explain::{Reason, SuggestedIndex, VerdictKind};
use curlybrace_core::guard::GuardFinding;
use curlybrace_core::store::{Namespace, Query};

fn finding(collection: &str, kind: VerdictKind) -> GuardFinding {
    GuardFinding {
        ns: Namespace::new("shop", collection),
        query: Query::default(),
        kind,
        reasons: vec![Reason::new("collscan")],
        suggestion: Some(SuggestedIndex {
            keys: vec![("status".into(), 1)],
        }),
    }
}

#[test]
fn each_finding_names_its_collection_and_problem() {
    let report = GuardReport {
        findings: vec![
            finding("orders", VerdictKind::CollectionScan),
            finding("users", VerdictKind::SortNotCovered),
        ],
        unchecked: 0,
    };
    let model = index_miss_model(&report, 2);
    assert_eq!(model.unchecked, 2);
    assert_eq!(model.findings.len(), 2);
    assert_eq!(model.findings[0].db, "shop");
    assert_eq!(model.findings[0].collection, "orders");
    assert_eq!(
        model.findings[0].warn.title_key,
        "query.guard.title_collection_scan"
    );
    assert_eq!(
        model.findings[1].warn.title_key,
        "query.guard.title_sort_not_covered"
    );
    assert!(
        model.findings[0]
            .warn
            .suggestion
            .as_deref()
            .is_some_and(|c| c.contains("createIndex"))
    );
}

#[test]
fn the_unchecked_line_is_singular_for_one_only() {
    assert_eq!(unchecked_key(1), "shell.guard.unchecked_one");
    assert_eq!(unchecked_key(3), "shell.guard.unchecked_other");
}
