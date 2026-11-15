//! `explain_pipeline`: the stages an aggregate explain sends.

use bson::doc;

use super::*;

fn st(text: &str) -> Stage {
    Stage {
        text: text.to_string(),
        enabled: true,
    }
}

fn off(text: &str) -> Stage {
    Stage {
        text: text.to_string(),
        enabled: false,
    }
}

#[test]
fn the_enabled_stages_up_to_the_selected_one_without_a_preview_limit() {
    let stages = [
        st(r#"{"$match": {"a": 1}}"#),
        off(r#"{"$sort": {"a": 1}}"#),
        st(r#"{"$group": {"_id": "$a"}}"#),
        st(r#"{"$project": {"x": 1}}"#),
    ];
    assert_eq!(
        explain_pipeline(&stages, 2).unwrap(),
        vec![
            doc! { "$match": { "a": 1 } },
            doc! { "$group": { "_id": "$a" } }
        ]
    );
}

#[test]
fn a_limit_the_author_wrote_stays() {
    let stages = [st(r#"{"$match": {}}"#), st(r#"{"$limit": 5}"#)];
    assert_eq!(
        explain_pipeline(&stages, 1).unwrap(),
        vec![doc! { "$match": {} }, doc! { "$limit": 5 }]
    );
}

#[test]
fn a_trailing_write_stage_is_dropped() {
    let stages = [st(r#"{"$match": {"a": 1}}"#), st(r#"{"$out": "t"}"#)];
    assert_eq!(
        explain_pipeline(&stages, 1).unwrap(),
        vec![doc! { "$match": { "a": 1 } }]
    );
}

#[test]
fn a_lone_write_stage_leaves_nothing_to_explain() {
    assert_eq!(
        explain_pipeline(&[st(r#"{"$merge": "t"}"#)], 0),
        Err(PipelineError::Empty)
    );
}

#[test]
fn a_write_stage_before_the_end_or_in_a_sub_pipeline_is_refused() {
    let middle = [st(r#"{"$out": "t"}"#), st(r#"{"$match": {}}"#)];
    assert!(matches!(
        explain_pipeline(&middle, 1),
        Err(PipelineError::WriteStageNotLast { index: 0, .. })
    ));
    let facet = [st(r#"{"$facet": {"f": [{"$out": "t"}]}}"#)];
    assert!(matches!(
        explain_pipeline(&facet, 0),
        Err(PipelineError::NestedWriteStage { index: 0, .. })
    ));
    let lookup = [st(
        r#"{"$lookup": {"from": "x", "pipeline": [{"$merge": "t"}], "as": "y"}}"#,
    )];
    assert!(matches!(
        explain_pipeline(&lookup, 0),
        Err(PipelineError::NestedWriteStage { index: 0, .. })
    ));
}

#[test]
fn a_stage_that_does_not_parse_and_an_empty_range_are_refused() {
    assert_eq!(
        explain_pipeline(&[st("{nope")], 0),
        Err(PipelineError::NotAStage { index: 0 })
    );
    assert_eq!(explain_pipeline(&[], 0), Err(PipelineError::Empty));
    assert_eq!(
        explain_pipeline(&[off(r#"{"$match": {}}"#)], 0),
        Err(PipelineError::Empty)
    );
}
