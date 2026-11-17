use bson::doc;

use super::*;
use crate::pipeline_options::{
    CollationOption, HintOption, PipelineOptions, ReadPreferenceMode, Strength,
};
use crate::pipeline_options_apply::MaxTimeEffect;
use crate::query::{next_page, prev_page};
use crate::runtime::COMMAND_TIMEOUT;
use crate::store::Query;

const OPS: [FindOperation; 3] = [
    FindOperation::Find,
    FindOperation::Explain,
    FindOperation::PlanCheck,
];

fn max_time(ms: u64) -> QueryOptions {
    PipelineOptions {
        max_time_ms: Some(ms),
        ..PipelineOptions::default()
    }
}

fn collation() -> CollationOption {
    CollationOption {
        locale: "en".into(),
        strength: Some(Strength::Secondary),
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    }
}

fn full() -> QueryOptions {
    PipelineOptions {
        collation: Some(collation()),
        hint: Some(HintOption::Name("status_1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(500),
        comment: Some("why".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    }
}

#[test]
fn the_default_query_has_no_options() {
    let q = Query::default();
    assert!(q.options.is_empty());
    assert_eq!(q.options, QueryOptions::default());
}

#[test]
fn the_default_applies_today_s_find() {
    let none = QueryOptions::default();
    let find = none.find_applied(FindOperation::Find);
    assert!(!find.send_max_time);
    assert!(find.is_default());
    assert!(find.options.collation.is_none());
    assert!(find.options.hint.is_none());
    assert!(find.options.comment.is_none());
    assert!(find.options.allow_disk_use.is_none());
    assert!(find.options.read_preference.is_none());
    for op in [FindOperation::Explain, FindOperation::PlanCheck] {
        let applied = none.find_applied(op);
        assert!(applied.send_max_time);
        assert_eq!(applied.options.max_time.as_millis(), 2000);
        assert!(applied.is_default());
        assert!(applied.options.collation.is_none());
        assert!(applied.options.read_preference.is_none());
    }
}

#[test]
fn find_carries_every_option_as_set() {
    for op in OPS {
        let applied = full().find_applied(op);
        assert_eq!(applied.options.collation, Some(collation()));
        assert_eq!(
            applied.options.hint,
            Some(HintOption::Name("status_1".into()))
        );
        assert_eq!(applied.options.comment.as_deref(), Some("why"));
        assert_eq!(
            applied.options.read_preference,
            Some(ReadPreferenceMode::SecondaryPreferred)
        );
        // Unlike an aggregate preview, a find sends the disk use as set.
        assert_eq!(applied.options.allow_disk_use, Some(true));
        assert!(!applied.is_default());
    }
    let off = PipelineOptions {
        allow_disk_use: Some(false),
        ..PipelineOptions::default()
    };
    assert_eq!(
        off.find_applied(FindOperation::Find).options.allow_disk_use,
        Some(false)
    );
}

#[test]
fn max_time_only_lowers_per_operation() {
    let ms = |o: &QueryOptions, op| o.find_applied(op).options.max_time.as_millis();
    let low = max_time(500);
    for op in OPS {
        assert_eq!(ms(&low, op), 500);
        assert_eq!(low.find_max_time_effect(op), MaxTimeEffect::Lowered);
        assert!(low.find_applied(op).max_time_lowered());
    }
    let mid = max_time(5000);
    assert_eq!(ms(&mid, FindOperation::Find), 5000);
    assert_eq!(ms(&mid, FindOperation::Explain), 2000);
    assert_eq!(ms(&mid, FindOperation::PlanCheck), 2000);
    assert_eq!(
        mid.find_max_time_effect(FindOperation::Explain),
        MaxTimeEffect::Capped
    );
    let high = max_time(45_000);
    assert_eq!(ms(&high, FindOperation::Find), 30_000);
    assert_eq!(
        high.find_max_time_effect(FindOperation::Find),
        MaxTimeEffect::Capped
    );
    assert_eq!(
        QueryOptions::default().find_max_time_effect(FindOperation::Find),
        MaxTimeEffect::Unchanged
    );
}

#[test]
fn a_set_time_on_find_is_sent() {
    assert!(
        max_time(500)
            .find_applied(FindOperation::Find)
            .send_max_time
    );
    // Even a value above the bound is sent, capped to it.
    assert!(
        max_time(45_000)
            .find_applied(FindOperation::Find)
            .send_max_time
    );
}

#[test]
fn the_find_bound_is_the_command_timeout() {
    assert_eq!(u128::from(FIND_MAX_TIME_MS), COMMAND_TIMEOUT.as_millis());
}

#[test]
fn the_explain_bound_is_today_s() {
    assert_eq!(
        FIND_EXPLAIN_MAX_TIME_MS as i64,
        crate::mongo_explain::EXPLAIN_MAX_TIME_MS
    );
}

#[test]
fn a_zero_time_is_raised_to_one_millisecond() {
    let applied = max_time(0).find_applied(FindOperation::Find);
    assert_eq!(applied.options.max_time.as_millis(), 1);
}

#[test]
fn query_debug_prints_no_option_value() {
    let q = Query {
        options: PipelineOptions {
            collation: Some(CollationOption {
                locale: "fr_CA".into(),
                ..collation()
            }),
            hint: Some(HintOption::Name("secret_idx".into())),
            comment: Some("secret-comment".into()),
            ..PipelineOptions::default()
        },
        ..Query::default()
    };
    let shown = format!("{q:?}");
    for secret in ["secret-comment", "secret_idx", "fr_CA"] {
        assert!(!shown.contains(secret), "{shown}");
    }
}

#[test]
fn next_and_prev_page_keep_the_options() {
    let q = Query {
        filter: doc! { "a": 1 },
        skip: 40,
        limit: 20,
        options: full(),
        ..Query::default()
    };
    assert_eq!(next_page(&q).options, q.options);
    assert_eq!(prev_page(&q).options, q.options);
}
