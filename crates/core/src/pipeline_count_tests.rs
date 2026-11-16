use bson::{Bson, doc};

use super::*;
use crate::error::CoreError;

fn stage(text: &str) -> Stage {
    Stage {
        text: text.to_string(),
        enabled: true,
    }
}

fn disabled(text: &str) -> Stage {
    Stage {
        text: text.to_string(),
        enabled: false,
    }
}

fn three() -> Vec<Stage> {
    vec![
        stage(r#"{"$match": {"a": 1}}"#),
        stage(r#"{"$group": {"_id": "$k"}}"#),
        stage(r#"{"$sort": {"_id": 1}}"#),
    ]
}

#[test]
fn the_cap_is_one_hundred_thousand() {
    assert_eq!(COUNT_CAP, 100_000);
}

#[test]
fn a_count_limits_then_counts_the_stages_up_to_the_index() {
    let pipeline = count_pipeline(&three(), 1, 50).unwrap();
    assert_eq!(
        pipeline,
        vec![
            doc! { "$match": { "a": 1 } },
            doc! { "$group": { "_id": "$k" } },
            doc! { "$limit": 50_i64 },
            doc! { "$count": "n" },
        ]
    );
}

#[test]
fn later_stages_are_not_part_of_an_earlier_count() {
    let pipeline = count_pipeline(&three(), 0, COUNT_CAP).unwrap();
    assert_eq!(pipeline.len(), 3);
    assert_eq!(pipeline[0], doc! { "$match": { "a": 1 } });
}

#[test]
fn disabled_stages_are_skipped() {
    let stages = vec![
        stage(r#"{"$match": {"a": 1}}"#),
        disabled(r#"{"$group": {"_id": "$k"}}"#),
        stage(r#"{"$sort": {"_id": 1}}"#),
    ];
    let pipeline = count_pipeline(&stages, 2, 10).unwrap();
    assert_eq!(pipeline.len(), 4);
    assert_eq!(pipeline[1], doc! { "$sort": { "_id": 1 } });
}

#[test]
fn counting_a_stage_that_is_a_write_stage_is_refused() {
    let stages = vec![stage(r#"{"$match": {}}"#), stage(r#"{"$out": "x"}"#)];
    assert!(count_pipeline(&stages, 0, 10).is_ok());
    assert_eq!(
        count_pipeline(&stages, 1, 10),
        Err(PipelineError::WriteStageNotLast {
            index: 1,
            name: "$out".into()
        })
    );
}

#[test]
fn a_write_stage_before_the_counted_stage_is_refused() {
    let stages = vec![
        stage(r#"{"$match": {}}"#),
        stage(r#"{"$merge": "x"}"#),
        stage(r#"{"$sort": {"a": 1}}"#),
    ];
    assert!(matches!(
        count_pipeline(&stages, 2, 10),
        Err(PipelineError::WriteStageNotLast { index: 1, .. })
    ));
}

#[test]
fn a_nested_write_stage_is_refused() {
    let stages = vec![stage(r#"{"$facet": {"f": [{"$out": "x"}]}}"#)];
    assert!(matches!(
        count_pipeline(&stages, 0, 10),
        Err(PipelineError::NestedWriteStage { index: 0, .. })
    ));
}

#[test]
fn a_stage_that_does_not_parse_is_refused() {
    let stages = vec![stage(r#"{"$match": {}}"#), stage("{oops")];
    assert_eq!(
        count_pipeline(&stages, 1, 10),
        Err(PipelineError::NotAStage { index: 1 })
    );
}

#[test]
fn nothing_enabled_is_empty() {
    assert_eq!(
        count_pipeline(&[disabled(r#"{"$match": {}}"#)], 0, 10),
        Err(PipelineError::Empty)
    );
}

#[test]
fn an_empty_result_counts_zero() {
    assert_eq!(count_of(&[]), Ok(0));
}

#[test]
fn the_count_is_read_from_n_in_any_numeric_type() {
    assert_eq!(count_of(&[doc! { "n": 7_i32 }]), Ok(7));
    assert_eq!(count_of(&[doc! { "n": 8_i64 }]), Ok(8));
    assert_eq!(count_of(&[doc! { "n": 9.0_f64 }]), Ok(9));
}

#[test]
fn anything_else_is_an_error() {
    let bad = [
        vec![doc! { "m": 1 }],
        vec![doc! { "n": "3" }],
        vec![doc! { "n": -1_i32 }],
        vec![doc! { "n": 1.5_f64 }],
        vec![doc! { "n": f64::NAN }],
        vec![doc! { "n": Bson::Null }],
        vec![doc! { "n": 1 }, doc! { "n": 2 }],
    ];
    for docs in bad {
        assert!(
            matches!(count_of(&docs), Err(CoreError::InvalidDocument(_))),
            "{docs:?}"
        );
    }
}

#[test]
fn counting_every_stage_at_once_equals_counting_each_one() {
    let cases: Vec<Vec<Stage>> = vec![
        three(),
        vec![
            stage(r#"{"$match": {"a": 1}}"#),
            disabled(r#"{"$group": {"_id": "$k"}}"#),
            stage(r#"{"$sort": {"_id": 1}}"#),
        ],
        vec![stage(r#"{"$match": {}}"#), stage(r#"{"$out": "x"}"#)],
        vec![
            stage(r#"{"$match": {}}"#),
            stage(r#"{"$merge": "x"}"#),
            stage(r#"{"$sort": {"a": 1}}"#),
        ],
        vec![
            stage(r#"{"$facet": {"f": [{"$out": "x"}]}}"#),
            stage(r#"{"$limit": 1}"#),
        ],
        vec![
            stage(r#"{"$match": {}}"#),
            stage("{oops"),
            stage(r#"{"$limit": 1}"#),
        ],
        vec![disabled(r#"{"$match": {}}"#), disabled("{oops")],
        vec![disabled("{oops"), stage(r#"{"$match": {}}"#)],
        Vec::new(),
    ];
    for stages in cases {
        let each: Vec<_> = (0..stages.len())
            .map(|upto| count_pipeline(&stages, upto, 10))
            .collect();
        assert_eq!(count_pipelines(&stages, 10), each, "{stages:?}");
    }
}

#[test]
fn a_write_stage_that_is_the_only_enabled_stage_is_refused_as_a_write_stage() {
    let cases = [
        (vec![stage(r#"{"$out": "x"}"#)], 0, "$out"),
        (
            vec![disabled(r#"{"$match": {}}"#), stage(r#"{"$merge": "x"}"#)],
            1,
            "$merge",
        ),
    ];
    for (stages, index, name) in cases {
        let upto = stages.len() - 1;
        let want = Err(PipelineError::WriteStageNotLast {
            index,
            name: name.to_string(),
        });
        assert_eq!(count_pipeline(&stages, upto, 10), want, "{stages:?}");
        assert_eq!(count_pipelines(&stages, 10)[upto], want, "{stages:?}");
    }
}
