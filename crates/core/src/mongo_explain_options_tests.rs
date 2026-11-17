//! What the explain commands send of a query's options: the same find, plus the options, with the
//! time limit only ever lowered below today's 2 s.

use bson::doc;
use mongodb::options::{ReadPreference, SelectionCriteria};

use super::*;
use crate::pipeline_options::{
    CollationOption, HintOption, PipelineOptions, ReadPreferenceMode, Strength,
};
use crate::query_options::FindOperation;
use crate::store::{Namespace, Query};

fn ns() -> Namespace {
    Namespace::new("db", "orders")
}

fn base() -> Query {
    Query {
        filter: doc! { "status": "open" },
        sort: Some(doc! { "created": -1 }),
        projection: Some(doc! { "status": 1 }),
        skip: 20,
        limit: 50,
        ..Query::default()
    }
}

fn with(options: PipelineOptions) -> Query {
    Query { options, ..base() }
}

fn command(q: &Query, verbosity: Verbosity, op: FindOperation) -> bson::Document {
    explain_command(&ns(), q, verbosity, &q.options.find_applied(op)).unwrap()
}

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "fr_CA".into(),
            strength: Some(Strength::Secondary),
            case_level: Some(true),
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        hint: Some(HintOption::Name("status_1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(500),
        comment: Some("why".into()),
        read_preference: Some(ReadPreferenceMode::Nearest),
    }
}

#[test]
fn the_default_explain_command_is_unchanged() {
    for (verbosity, name) in [
        (Verbosity::ExecutionStats, "executionStats"),
        (Verbosity::QueryPlanner, "queryPlanner"),
    ] {
        let expected = doc! {
            "explain": {
                "find": "orders",
                "filter": { "status": "open" },
                "sort": { "created": -1 },
                "projection": { "status": 1 },
                "skip": 20_i64,
                "limit": 51_i64,
            },
            "verbosity": name,
            "maxTimeMS": 2000_i64,
        };
        let q = base();
        assert_eq!(command(&q, verbosity, FindOperation::Explain), expected);
        assert_eq!(command(&q, verbosity, FindOperation::PlanCheck), expected);
    }
}

#[test]
fn the_explain_find_carries_collation_hint_comment_and_disk_use() {
    let cmd = command(
        &with(every_option()),
        Verbosity::ExecutionStats,
        FindOperation::Explain,
    );
    let find = cmd.get_document("explain").unwrap();
    assert_eq!(
        find.get_document("collation").unwrap(),
        &doc! { "locale": "fr_CA", "strength": 2, "caseLevel": true }
    );
    assert_eq!(find.get_str("hint").unwrap(), "status_1");
    assert!(find.get_bool("allowDiskUse").unwrap());
    assert_eq!(find.get_str("comment").unwrap(), "why");
    // The read preference is where the command runs, never a field of the command.
    assert!(!find.contains_key("readPreference"));
    assert!(!cmd.contains_key("readPreference"));
}

#[test]
fn a_key_hint_goes_in_as_a_document() {
    let q = with(PipelineOptions {
        hint: Some(HintOption::Keys(doc! { "status": 1, "total": -1 })),
        ..PipelineOptions::default()
    });
    let cmd = command(&q, Verbosity::QueryPlanner, FindOperation::PlanCheck);
    assert_eq!(
        cmd.get_document("explain")
            .unwrap()
            .get_document("hint")
            .unwrap(),
        &doc! { "status": 1, "total": -1 }
    );
}

#[test]
fn the_explain_time_only_lowers() {
    let time = |ms: u64, op| {
        let q = with(PipelineOptions {
            max_time_ms: Some(ms),
            ..PipelineOptions::default()
        });
        command(&q, Verbosity::ExecutionStats, op)
            .get_i64("maxTimeMS")
            .unwrap()
    };
    for op in [FindOperation::Explain, FindOperation::PlanCheck] {
        assert_eq!(time(500, op), 500);
        assert_eq!(time(5000, op), 2000);
        assert_eq!(time(60_000, op), 2000);
    }
}

#[test]
fn explain_criteria_prefer_the_option_over_the_connection() {
    let applied = every_option().find_applied(FindOperation::Explain);
    let chosen = read_criteria(&applied.options);
    assert!(matches!(
        chosen,
        Some(SelectionCriteria::ReadPreference(
            ReadPreference::Nearest { .. }
        ))
    ));
    let none = PipelineOptions::default().find_applied(FindOperation::Explain);
    assert!(read_criteria(&none.options).is_none());
}

#[tokio::test]
async fn the_connection_preference_is_the_fallback_when_no_option_is_set() {
    let db = mongodb::Client::with_uri_str("mongodb://127.0.0.1:1/?readPreference=secondary")
        .await
        .expect("uri parses")
        .database("db");
    let none = PipelineOptions::default().find_applied(FindOperation::Explain);
    assert!(matches!(
        criteria_for(&db, &none),
        Some(SelectionCriteria::ReadPreference(
            ReadPreference::Secondary { .. }
        ))
    ));
    let set = every_option().find_applied(FindOperation::Explain);
    assert!(matches!(
        criteria_for(&db, &set),
        Some(SelectionCriteria::ReadPreference(
            ReadPreference::Nearest { .. }
        ))
    ));
}
