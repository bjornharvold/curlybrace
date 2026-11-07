use super::*;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{Reason, SuggestedIndex, Verdict, VerdictKind};
use curlybrace_core::store::{Namespace, Query};

fn verdict(kind: VerdictKind) -> Verdict {
    Verdict {
        kind,
        index_name: None,
        keys_examined: 0,
        docs_examined: 0,
        returned: 0,
        plan_only: false,
        reasons: Vec::new(),
        suggestion: None,
        namespace: Some("shop.orders".into()),
    }
}

fn orders() -> Namespace {
    Namespace::new("shop", "orders")
}

fn ready(verdict: Verdict) -> VerdictState {
    VerdictState::Ready {
        query: Query::default(),
        verdict,
    }
}

fn model(kind: VerdictKind) -> BadgeModel {
    badge(&ready(verdict(kind)), Some(&orders())).expect("a ready verdict has a badge")
}

#[test]
fn badge_for_each_kind_has_distinct_title_and_tone() {
    let kinds = [
        (VerdictKind::IndexUsed, Tone::Good),
        (VerdictKind::CoveredByIndex, Tone::Good),
        (VerdictKind::CollectionScan, Tone::Warn),
        (VerdictKind::SortNotCovered, Tone::Warn),
        (VerdictKind::InefficientIndex, Tone::Warn),
        (VerdictKind::NoFilter, Tone::Neutral),
        (VerdictKind::EmptyCollection, Tone::Neutral),
    ];
    let mut titles = Vec::new();
    for (kind, tone) in kinds {
        let m = model(kind);
        assert_eq!(m.tone, tone, "{kind:?}");
        assert!(!m.icon.is_empty(), "{kind:?}");
        titles.push(m.title_key);
    }
    let mut unique = titles.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), titles.len(), "{titles:?}");
}

#[test]
fn none_has_no_badge() {
    assert_eq!(badge(&VerdictState::None, Some(&orders())), None);
}

#[test]
fn pending_is_checking() {
    let m = badge(
        &VerdictState::Pending {
            query: Query::default(),
        },
        Some(&orders()),
    )
    .unwrap();
    assert_eq!(m.tone, Tone::Checking);
    assert_eq!(m.title_key, "query.verdict.checking");
    assert!(m.suggestion.is_none() && !m.plan_only);
}

fn unavailable(error: CoreError) -> BadgeModel {
    badge(
        &VerdictState::Unavailable {
            query: Query::default(),
            error,
        },
        Some(&orders()),
    )
    .unwrap()
}

#[test]
fn unavailable_shows_reason_key_for_unsupported_and_database_errors() {
    let unsupported = unavailable(CoreError::Unsupported("view: secret detail".into()));
    let database = unavailable(CoreError::Database("boom 10.0.0.1".into()));
    for m in [&unsupported, &database] {
        assert_eq!(m.tone, Tone::Unavailable);
        assert_eq!(m.title_key, "query.verdict.unavailable");
        assert_eq!(m.reasons.len(), 1);
    }
    assert_eq!(unsupported.reasons[0].key, "unsupported");
    assert_eq!(database.reasons[0].key, "unavailable");
    assert!(unsupported.reasons[0].params.is_empty());
    assert!(database.reasons[0].params.is_empty());
}

#[test]
fn suggestion_command_names_database_and_collection() {
    let mut v = verdict(VerdictKind::CollectionScan);
    v.reasons = vec![Reason::new("collscan")];
    v.suggestion = Some(SuggestedIndex {
        keys: vec![("status".into(), 1)],
    });
    let m = badge(&ready(v), Some(&orders())).unwrap();
    assert_eq!(
        m.suggestion.as_deref(),
        Some("db.getSiblingDB(\"shop\").getCollection(\"orders\").createIndex({ \"status\": 1 })")
    );
    assert_eq!(m.reasons[0].key, "collscan");
}

#[test]
fn a_good_verdict_offers_no_suggestion() {
    let mut v = verdict(VerdictKind::IndexUsed);
    v.suggestion = Some(SuggestedIndex {
        keys: vec![("a".into(), 1)],
    });
    assert_eq!(badge(&ready(v), Some(&orders())).unwrap().suggestion, None);
}

#[test]
fn plan_only_adds_the_estimate_note() {
    let mut v = verdict(VerdictKind::CollectionScan);
    v.plan_only = true;
    assert!(badge(&ready(v), Some(&orders())).unwrap().plan_only);
    assert!(!model(VerdictKind::CollectionScan).plan_only);
}

/// Every `{name}` placeholder of `template`.
fn placeholders(template: &str) -> Vec<String> {
    template
        .split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string()))
        .collect()
}

/// The reasons core really emits: `classify` over every fixture, with and without a filter.
fn emitted_reasons() -> Vec<Reason> {
    use curlybrace_core::ejson::editor_json_to_doc;
    use curlybrace_core::explain::classify;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../core/src/explain/fixtures");
    let queries = [
        Query {
            filter: bson::doc! { "status": "open" },
            ..Query::default()
        },
        Query::default(),
    ];
    let mut reasons = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap();
        let explain = editor_json_to_doc(&text).unwrap();
        for query in &queries {
            let verdict = classify(&explain, query).unwrap();
            reasons.extend(verdict.reasons);
        }
    }
    reasons
}

#[test]
fn every_reason_core_really_emits_has_a_resolvable_string_with_supplied_params() {
    let i18n = crate::I18n::english();
    let mut reasons = emitted_reasons();
    for error in [
        CoreError::Unsupported("x".into()),
        CoreError::Database("x".into()),
    ] {
        reasons.extend(unavailable(error).reasons);
    }
    assert!(reasons.len() >= 5, "fixtures should cover many reasons");
    for reason in reasons {
        let key = reason_text_key(reason.key)
            .unwrap_or_else(|| panic!("core emits `{}` but the badge has no string", reason.key));
        let template = i18n.t(key);
        assert!(
            !template.starts_with('\u{27e6}'),
            "{key} missing from en.json"
        );
        for name in placeholders(&template) {
            assert!(
                reason.params.iter().any(|(n, _)| *n == name),
                "{key} uses {{{name}}} but core does not supply it"
            );
        }
    }
}

#[test]
fn no_collection_selected_means_no_suggestion() {
    let mut v = verdict(VerdictKind::CollectionScan);
    v.suggestion = Some(SuggestedIndex {
        keys: vec![("status".into(), 1)],
    });
    assert_eq!(badge(&ready(v), None).unwrap().suggestion, None);
}

fn scan_with_suggestion(planned: Option<&str>) -> Verdict {
    let mut v = verdict(VerdictKind::CollectionScan);
    v.suggestion = Some(SuggestedIndex {
        keys: vec![("status".into(), 1)],
    });
    v.namespace = planned.map(str::to_owned);
    v
}

#[test]
fn a_view_gets_no_suggestion_because_the_plan_ran_on_its_source() {
    let v = scan_with_suggestion(Some("shop.orders_source"));
    assert_eq!(badge(&ready(v), Some(&orders())).unwrap().suggestion, None);
}

#[test]
fn a_matching_planned_namespace_keeps_the_suggestion() {
    let v = scan_with_suggestion(Some("shop.orders"));
    assert!(
        badge(&ready(v), Some(&orders()))
            .unwrap()
            .suggestion
            .is_some()
    );
}

#[test]
fn an_unknown_planned_namespace_keeps_the_suggestion() {
    let v = scan_with_suggestion(None);
    assert!(
        badge(&ready(v), Some(&orders()))
            .unwrap()
            .suggestion
            .is_some()
    );
}
