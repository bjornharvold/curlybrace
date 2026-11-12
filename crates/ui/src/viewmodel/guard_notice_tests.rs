use super::*;
use curlybrace_core::explain::{Reason, SuggestedIndex, VerdictKind};
use curlybrace_core::guard::{GuardFinding, GuardReport};
use curlybrace_core::store::{Namespace, Query};

fn finding(kind: VerdictKind, suggested: bool) -> GuardFinding {
    GuardFinding {
        ns: Namespace::new("shop", "orders"),
        query: Query::default(),
        kind,
        reasons: vec![Reason::new("collscan")],
        suggestion: suggested.then(|| SuggestedIndex {
            keys: vec![("status".into(), 1)],
        }),
    }
}

fn warn(findings: Vec<GuardFinding>) -> GuardState {
    GuardState::Warn {
        query: Query::default(),
        report: GuardReport {
            findings,
            unchecked: 0,
        },
    }
}

fn warning(state: &GuardState) -> WarnModel {
    match guard_notice(state) {
        Some(GuardNotice::Warn(model)) => model,
        other => panic!("expected a warning, got {other:?}"),
    }
}

#[test]
fn nothing_to_show_without_a_guard_state() {
    assert_eq!(guard_notice(&GuardState::None), None);
}

#[test]
fn checking_is_a_line_without_actions() {
    let state = GuardState::Checking {
        request: 1,
        query: Query::default(),
    };
    assert_eq!(guard_notice(&state), Some(GuardNotice::Checking));
}

#[test]
fn a_scan_names_its_problem_reasons_and_suggested_command() {
    let model = warning(&warn(vec![finding(VerdictKind::CollectionScan, true)]));
    assert_eq!(model.title_key, "query.guard.title_collection_scan");
    assert_eq!(model.reasons, vec![Reason::new("collscan")]);
    assert_eq!(
        model.suggestion.as_deref(),
        Some("db.getSiblingDB(\"shop\").getCollection(\"orders\").createIndex({ \"status\": 1 })")
    );
}

#[test]
fn a_blocking_sort_has_its_own_title_and_no_suggestion_when_none_is_given() {
    let model = warning(&warn(vec![finding(VerdictKind::SortNotCovered, false)]));
    assert_eq!(model.title_key, "query.guard.title_sort_not_covered");
    assert_eq!(model.suggestion, None);
}

#[test]
fn only_the_first_finding_is_shown() {
    let model = warning(&warn(vec![
        finding(VerdictKind::SortNotCovered, false),
        finding(VerdictKind::CollectionScan, true),
    ]));
    assert_eq!(model.title_key, "query.guard.title_sort_not_covered");
}

#[test]
fn a_warning_without_findings_shows_nothing() {
    assert_eq!(guard_notice(&warn(Vec::new())), None);
}

#[test]
fn a_kind_the_guard_does_not_raise_still_gets_a_title() {
    let model = warning(&warn(vec![finding(VerdictKind::InefficientIndex, false)]));
    assert_eq!(model.title_key, "query.verdict.title_inefficient");
}
