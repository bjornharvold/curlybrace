use bson::doc;

use super::*;
use crate::error::CoreError;

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
fn syntax_error_carries_line_and_column() {
    let err = parse_pipeline_text("[\n  {\"$match\": }\n]").unwrap_err();
    match err {
        PipelineError::Syntax { line, column, .. } => {
            assert_eq!(line, 2);
            assert!(column > 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn not_an_array_is_refused() {
    for text in [r#"{"$match": {}}"#, "1", r#""x""#, "null"] {
        assert_eq!(
            parse_pipeline_text(text).unwrap_err(),
            PipelineError::NotAnArray
        );
    }
}

#[test]
fn element_that_is_not_an_object_is_refused() {
    let err = parse_pipeline_text(r#"[{"$match": {}}, 3]"#).unwrap_err();
    assert_eq!(err, PipelineError::NotAStage { index: 1 });
}

#[test]
fn text_parses_to_pretty_enabled_stages() {
    let stages = parse_pipeline_text(r#"[{"$match":{"a":1}},{"$limit":5}]"#).unwrap();
    assert_eq!(stages.len(), 2);
    assert!(stages.iter().all(|s| s.enabled));
    assert_eq!(stages[0].text, "{\n  \"$match\": {\n    \"a\": 1\n  }\n}");
    assert_eq!(stage_name(&stages[1].text).as_deref(), Some("$limit"));
}

#[test]
fn empty_array_parses_to_no_stages() {
    assert!(parse_pipeline_text("  [ ]  ").unwrap().is_empty());
}

#[test]
fn text_round_trips() {
    let stages = parse_pipeline_text(r#"[{"$match":{"a":1}},{"$sort":{"a":-1}}]"#).unwrap();
    let text = pipeline_text(&stages);
    assert_eq!(parse_pipeline_text(&text).unwrap(), stages);
}

#[test]
fn text_mode_shows_disabled_stages_and_loses_the_flag() {
    let stages = vec![st(r#"{"$match": {}}"#), off(r#"{"$limit": 1}"#)];
    let text = pipeline_text(&stages);
    let back = parse_pipeline_text(&text).unwrap();
    assert_eq!(back.len(), 2);
    assert!(back.iter().all(|s| s.enabled));
}

#[test]
fn unparsable_stage_text_is_kept_so_its_error_shows() {
    let text = pipeline_text(&[st("{ not json")]);
    assert!(matches!(
        parse_pipeline_text(&text),
        Err(PipelineError::Syntax { .. })
    ));
}

#[test]
fn stage_name_reads_the_single_dollar_key() {
    assert_eq!(
        stage_name(r#"{"$group": {"_id": 1}}"#).as_deref(),
        Some("$group")
    );
    assert_eq!(stage_name("{ bad"), None);
    assert_eq!(stage_name(r#"{"a": 1}"#), None);
    assert_eq!(stage_name(r#"{"$a": 1, "$b": 2}"#), None);
    assert_eq!(stage_name("[1]"), None);
}

#[test]
fn plan_appends_limit_last() {
    let plan = preview_pipeline(&[st(r#"{"$match": {"a": 1}}"#)], 0, 20).unwrap();
    assert_eq!(
        plan.pipeline,
        vec![doc! {"$match": {"a": 1}}, doc! {"$limit": 20_i64}]
    );
    assert!(plan.not_run.is_empty());
}

#[test]
fn plan_parses_extended_json() {
    let plan = preview_pipeline(
        &[st(
            r#"{"$match": {"_id": {"$oid": "507f1f77bcf86cd799439011"}}}"#,
        )],
        0,
        5,
    )
    .unwrap();
    let id = plan.pipeline[0]
        .get_document("$match")
        .unwrap()
        .get("_id")
        .unwrap();
    assert!(matches!(id, bson::Bson::ObjectId(_)));
}

#[test]
fn plan_respects_upto_and_skips_disabled() {
    let stages = vec![
        st(r#"{"$match": {"a": 1}}"#),
        off(r#"{"$sort": {"a": 1}}"#),
        st(r#"{"$project": {"a": 1}}"#),
        st(r#"{"$skip": 3}"#),
    ];
    let plan = preview_pipeline(&stages, 2, 7).unwrap();
    assert_eq!(
        plan.pipeline,
        vec![
            doc! {"$match": {"a": 1}},
            doc! {"$project": {"a": 1}},
            doc! {"$limit": 7_i64}
        ]
    );
}

#[test]
fn upto_past_the_end_takes_everything() {
    let plan = preview_pipeline(&[st(r#"{"$match": {}}"#)], 99, 1).unwrap();
    assert_eq!(plan.pipeline.len(), 2);
}

#[test]
fn empty_and_all_disabled_are_errors() {
    assert_eq!(
        preview_pipeline(&[], 0, 20).unwrap_err(),
        PipelineError::Empty
    );
    assert_eq!(
        preview_pipeline(&[off(r#"{"$match": {}}"#)], 0, 20).unwrap_err(),
        PipelineError::Empty
    );
    // Only the stage past `upto` is enabled.
    let stages = vec![off(r#"{"$match": {}}"#), st(r#"{"$limit": 1}"#)];
    assert_eq!(
        preview_pipeline(&stages, 0, 20).unwrap_err(),
        PipelineError::Empty
    );
}

#[test]
fn a_stage_must_be_one_dollar_key_object() {
    for bad in [
        r#"{"$match": {}, "$limit": 1}"#,
        r#"{"match": {}}"#,
        "{}",
        "[1]",
        "3",
        "{ nope",
        "",
    ] {
        assert_eq!(
            preview_pipeline(&[st(r#"{"$match": {}}"#), st(bad)], 1, 20).unwrap_err(),
            PipelineError::NotAStage { index: 1 },
            "{bad}"
        );
    }
}

#[test]
fn trailing_write_stages_are_dropped_and_named() {
    for name in ["$out", "$merge"] {
        let stages = vec![
            st(r#"{"$match": {"a": 1}}"#),
            st(&format!(r#"{{"{name}": "target"}}"#)),
        ];
        let plan = preview_pipeline(&stages, 1, 20).unwrap();
        assert_eq!(plan.not_run, vec![name.to_string()]);
        assert_eq!(
            plan.pipeline,
            vec![doc! {"$match": {"a": 1}}, doc! {"$limit": 20_i64}]
        );
        assert_eq!(contains_write_stage(&plan.pipeline), None);
    }
}

#[test]
fn trailing_write_stage_ignores_disabled_stages_after_it() {
    let stages = vec![
        st(r#"{"$match": {}}"#),
        st(r#"{"$out": "t"}"#),
        off(r#"{"$limit": 1}"#),
    ];
    let plan = preview_pipeline(&stages, 2, 20).unwrap();
    assert_eq!(plan.not_run, vec!["$out".to_string()]);
}

#[test]
fn write_stage_only_is_empty_after_dropping() {
    assert_eq!(
        preview_pipeline(&[st(r#"{"$out": "t"}"#)], 0, 20).unwrap_err(),
        PipelineError::Empty
    );
}

#[test]
fn write_stage_in_the_middle_is_refused() {
    let stages = vec![st(r#"{"$merge": {"into": "t"}}"#), st(r#"{"$match": {}}"#)];
    assert_eq!(
        preview_pipeline(&stages, 1, 20).unwrap_err(),
        PipelineError::WriteStageNotLast {
            index: 0,
            name: "$merge".into()
        }
    );
}

#[test]
fn write_stage_beyond_upto_does_not_matter() {
    let stages = vec![st(r#"{"$match": {}}"#), st(r#"{"$out": "t"}"#)];
    let plan = preview_pipeline(&stages, 0, 20).unwrap();
    assert!(plan.not_run.is_empty());
}

#[test]
fn nested_write_stages_are_refused() {
    let cases = [
        r#"{"$facet": {"a": [{"$match": {}}, {"$out": "t"}]}}"#,
        r#"{"$lookup": {"from": "c", "as": "x", "pipeline": [{"$merge": {"into": "t"}}]}}"#,
        r#"{"$unionWith": {"coll": "c", "pipeline": [{"$out": "t"}]}}"#,
        r#"{"$facet": {"a": [{"$lookup": {"from": "c", "as": "x", "pipeline": [{"$out": "t"}]}}]}}"#,
    ];
    for case in cases {
        let err = preview_pipeline(&[st(case)], 0, 20).unwrap_err();
        assert!(
            matches!(err, PipelineError::NestedWriteStage { index: 0, .. }),
            "{case}: {err:?}"
        );
    }
}

#[test]
fn contains_write_stage_finds_any_depth() {
    assert_eq!(contains_write_stage(&[doc! {"$match": {}}]), None);
    assert_eq!(
        contains_write_stage(&[doc! {"$out": "t"}]).as_deref(),
        Some("$out")
    );
    let nested = doc! {"$facet": {"a": [{"$match": {}}, {"$merge": {"into": "t"}}]}};
    assert_eq!(contains_write_stage(&[nested]).as_deref(), Some("$merge"));
}

#[test]
fn deeply_nested_json_never_panics() {
    let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
    let obj = format!(r#"{{"$match": {{"a": {deep}}}}}"#);
    let _ = preview_pipeline(&[st(&obj)], 0, 20);
    let text = format!(r#"[{{"$match": {{"a": {deep}}}}}]"#);
    assert!(parse_pipeline_text(&text).is_err());
    let nested_objs = format!("{}1{}", r#"{"a":"#.repeat(200), "}".repeat(200));
    let _ = preview_pipeline(&[st(&format!(r#"{{"$match": {nested_objs}}}"#))], 0, 20);
}

#[test]
fn errors_convert_to_the_core_taxonomy() {
    let syntax = PipelineError::Syntax {
        line: 2,
        column: 4,
        message: "boom".into(),
    };
    match syntax.into_core() {
        CoreError::InvalidJson(m) => assert!(m.contains("line 2") && m.contains("column 4")),
        other => panic!("{other:?}"),
    }
    for e in [
        PipelineError::NotAnArray,
        PipelineError::NotAStage { index: 0 },
        PipelineError::WriteStageNotLast {
            index: 0,
            name: "$out".into(),
        },
        PipelineError::NestedWriteStage {
            index: 0,
            name: "$out".into(),
        },
        PipelineError::Empty,
    ] {
        assert!(!e.to_string().is_empty());
        assert!(matches!(e.into_core(), CoreError::InvalidDocument(_)));
    }
}
