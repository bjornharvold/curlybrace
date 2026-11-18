use std::time::Duration;

use bson::{Bson, doc};
use mongodb::options::{Hint, ReadPreference, SelectionCriteria};

use super::*;
use crate::pipeline_options::{
    CollationOption, HintOption, PipelineOptions, ReadPreferenceMode, Strength,
};
use crate::query_options::FindOperation;
use crate::store::Query;

fn sent(q: &Query) -> mongodb::options::FindOptions {
    find_options(q, &q.options.find_applied(FindOperation::Find)).expect("options build")
}

fn paged() -> Query {
    Query {
        filter: doc! { "status": "A" },
        sort: Some(doc! { "total": -1 }),
        projection: Some(doc! { "status": 1 }),
        skip: 40,
        limit: 20,
        ..Query::default()
    }
}

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: Some(Strength::Secondary),
            case_level: None,
            case_first: None,
            numeric_ordering: Some(true),
            alternate: None,
        }),
        hint: Some(HintOption::Keys(doc! { "status": 1, "total": -1 })),
        allow_disk_use: Some(true),
        max_time_ms: Some(500),
        comment: Some("nightly".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    }
}

#[test]
fn default_find_options_are_today_s() {
    let o = sent(&paged());
    assert_eq!(o.skip, Some(40));
    assert_eq!(o.limit, Some(21));
    assert_eq!(o.sort, Some(doc! { "total": -1 }));
    assert_eq!(o.projection, Some(doc! { "status": 1 }));
    assert!(o.max_time.is_none());
    assert!(o.allow_disk_use.is_none());
    assert!(o.collation.is_none());
    assert!(o.hint.is_none());
    assert!(o.comment.is_none());
    assert!(o.selection_criteria.is_none());
}

#[test]
fn a_zero_skip_is_still_sent_like_before() {
    assert_eq!(sent(&Query::default()).skip, Some(0));
}

#[test]
fn an_unlimited_query_sends_no_limit() {
    let q = Query {
        limit: 0,
        ..Query::default()
    };
    assert_eq!(sent(&q).limit, None);
}

#[test]
fn find_options_carry_every_set_option() {
    let q = Query {
        options: every_option(),
        ..paged()
    };
    let o = sent(&q);
    let collation = o.collation.expect("collation");
    assert_eq!(
        bson::to_document(&collation).unwrap(),
        doc! { "locale": "en", "strength": 2, "numericOrdering": true }
    );
    assert_eq!(o.hint, Some(Hint::Keys(doc! { "status": 1, "total": -1 })));
    assert_eq!(o.comment, Some(Bson::String("nightly".into())));
    assert_eq!(o.max_time, Some(Duration::from_millis(500)));
    assert_eq!(o.allow_disk_use, Some(true));
    assert!(matches!(
        o.selection_criteria,
        Some(SelectionCriteria::ReadPreference(
            ReadPreference::SecondaryPreferred { .. }
        ))
    ));
}

#[test]
fn a_hint_by_name_is_sent_by_name() {
    let q = Query {
        options: PipelineOptions {
            hint: Some(HintOption::Name("status_1".into())),
            ..PipelineOptions::default()
        },
        ..Query::default()
    };
    assert_eq!(sent(&q).hint, Some(Hint::Name("status_1".into())));
}

#[test]
fn a_time_above_the_bound_is_capped_at_thirty_seconds() {
    let q = Query {
        options: PipelineOptions {
            max_time_ms: Some(45_000),
            ..PipelineOptions::default()
        },
        ..Query::default()
    };
    assert_eq!(sent(&q).max_time, Some(Duration::from_secs(30)));
}

#[test]
fn a_negative_skip_still_fails_before_any_option() {
    let q = Query {
        skip: u64::MAX,
        options: every_option(),
        ..Query::default()
    };
    let applied = q.options.find_applied(FindOperation::Find);
    assert!(find_options(&q, &applied).is_err());
}
