use bson::doc;

use crate::error::CoreError;
use crate::pipeline::{
    PipelineError, RunTarget, Stage, ensure_run_shape, run_pipeline, run_target,
};

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

fn target(db: Option<&str>, collection: &str) -> RunTarget {
    RunTarget {
        db: db.map(str::to_string),
        collection: collection.to_string(),
    }
}

fn plan_target(last: &str) -> Result<RunTarget, PipelineError> {
    run_pipeline(&[st(r#"{"$match": {"a": 1}}"#), st(last)]).map(|p| p.target)
}

#[test]
fn every_out_and_merge_target_form_resolves() {
    let cases = [
        (r#"{"$out": "dest"}"#, "$out", target(None, "dest")),
        (
            r#"{"$out": {"db": "other", "coll": "dest"}}"#,
            "$out",
            target(Some("other"), "dest"),
        ),
        (r#"{"$merge": "dest"}"#, "$merge", target(None, "dest")),
        (
            r#"{"$merge": {"into": "dest", "whenMatched": "replace"}}"#,
            "$merge",
            target(None, "dest"),
        ),
        (
            r#"{"$merge": {"into": {"db": "other", "coll": "dest"}}}"#,
            "$merge",
            target(Some("other"), "dest"),
        ),
    ];
    for (text, stage, expected) in cases {
        let plan = run_pipeline(&[st(r#"{"$match": {}}"#), st(text)]).expect(text);
        assert_eq!(plan.stage, stage, "{text}");
        assert_eq!(plan.target, expected, "{text}");
    }
}

#[test]
fn an_unusable_target_is_a_refusal_at_the_stage_index() {
    let bad = [
        r#"{"$out": ""}"#,
        r#"{"$out": 5}"#,
        r#"{"$out": {}}"#,
        r#"{"$out": {"db": "x"}}"#,
        r#"{"$out": {"db": "", "coll": "c"}}"#,
        r#"{"$out": {"db": 1, "coll": "c"}}"#,
        r#"{"$merge": ""}"#,
        r#"{"$merge": {}}"#,
        r#"{"$merge": {"into": ""}}"#,
        r#"{"$merge": {"into": 3}}"#,
        r#"{"$merge": {"into": {"db": "x"}}}"#,
        r#"{"$merge": {"whenMatched": "replace"}}"#,
        r#"{"$merge": 1}"#,
    ];
    for text in bad {
        assert_eq!(
            plan_target(text),
            Err(PipelineError::TargetUnresolved { index: 1 }),
            "{text}"
        );
    }
}

#[test]
fn the_plan_holds_every_enabled_stage_and_adds_no_limit() {
    let plan = run_pipeline(&[
        st(r#"{"$match": {"a": 1}}"#),
        off(r#"{"$sort": {"a": 1}}"#),
        st(r#"{"$project": {"a": 1}}"#),
        st(r#"{"$out": "dest"}"#),
    ])
    .unwrap();
    assert_eq!(
        plan.pipeline,
        vec![
            doc! { "$match": { "a": 1 } },
            doc! { "$project": { "a": 1 } },
            doc! { "$out": "dest" },
        ]
    );
}

#[test]
fn a_write_stage_alone_is_a_valid_run() {
    let plan = run_pipeline(&[st(r#"{"$merge": "dest"}"#)]).unwrap();
    assert_eq!(plan.pipeline.len(), 1);
}

#[test]
fn a_write_stage_that_is_not_last_is_refused() {
    let err = run_pipeline(&[
        st(r#"{"$out": "a"}"#),
        st(r#"{"$match": {}}"#),
        st(r#"{"$out": "b"}"#),
    ])
    .unwrap_err();
    assert_eq!(
        err,
        PipelineError::WriteStageNotLast {
            index: 0,
            name: "$out".into()
        }
    );
}

#[test]
fn two_write_stages_are_refused() {
    let err = run_pipeline(&[st(r#"{"$out": "a"}"#), st(r#"{"$merge": "b"}"#)]).unwrap_err();
    assert!(matches!(
        err,
        PipelineError::WriteStageNotLast { index: 0, .. }
    ));
}

#[test]
fn a_nested_write_stage_is_refused_in_every_sub_pipeline() {
    let nested = [
        r#"{"$facet": {"f": [{"$out": "x"}]}}"#,
        r#"{"$lookup": {"from": "c", "pipeline": [{"$merge": "x"}], "as": "j"}}"#,
        r#"{"$unionWith": {"coll": "c", "pipeline": [{"$out": "x"}]}}"#,
    ];
    for text in nested {
        let err = run_pipeline(&[st(text), st(r#"{"$out": "dest"}"#)]).unwrap_err();
        assert!(
            matches!(err, PipelineError::NestedWriteStage { index: 0, .. }),
            "{text}: {err:?}"
        );
    }
}

#[test]
fn no_enabled_stage_is_empty_and_a_missing_write_stage_is_named() {
    assert_eq!(run_pipeline(&[]), Err(PipelineError::Empty));
    assert_eq!(
        run_pipeline(&[off(r#"{"$out": "a"}"#)]),
        Err(PipelineError::Empty)
    );
    assert_eq!(
        run_pipeline(&[st(r#"{"$match": {}}"#)]),
        Err(PipelineError::NoWriteStage)
    );
}

#[test]
fn a_disabled_trailing_write_stage_means_no_write_stage() {
    let err = run_pipeline(&[st(r#"{"$match": {}}"#), off(r#"{"$out": "dest"}"#)]).unwrap_err();
    assert_eq!(err, PipelineError::NoWriteStage);
}

#[test]
fn an_unparsable_stage_is_not_a_stage() {
    assert_eq!(
        run_pipeline(&[st("{\"$match\": "), st(r#"{"$out": "d"}"#)]),
        Err(PipelineError::NotAStage { index: 0 })
    );
}

#[test]
fn run_target_reads_the_last_stage_of_a_pipeline() {
    let p = vec![doc! { "$match": {} }, doc! { "$merge": { "into": "t" } }];
    assert_eq!(
        run_target(&p),
        Some(("$merge".to_string(), target(None, "t")))
    );
    assert_eq!(run_target(&[doc! { "$match": {} }]), None);
    assert_eq!(run_target(&[]), None);
}

fn source() -> crate::store::Namespace {
    crate::store::Namespace::new("app", "src")
}

fn shape_message(pipeline: &[bson::Document]) -> String {
    match ensure_run_shape(&source(), pipeline) {
        Err(CoreError::InvalidDocument(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_good_shape_passes() {
    assert!(ensure_run_shape(&source(), &[doc! { "$match": {} }, doc! { "$out": "dest" }]).is_ok());
    assert!(ensure_run_shape(&source(), &[doc! { "$merge": { "into": "dest" } }]).is_ok());
}

#[test]
fn a_bad_shape_is_refused_naming_the_stage_never_the_text() {
    let cases: Vec<Vec<bson::Document>> = vec![
        vec![],
        vec![doc! { "$match": { "secret": "hunter2" } }],
        vec![doc! { "$out": "secret_dest" }, doc! { "$match": {} }],
        vec![doc! { "$out": "secret_a" }, doc! { "$merge": "secret_b" }],
        vec![
            doc! { "$facet": { "f": [ { "$out": "secret_x" } ] } },
            doc! { "$out": "secret_dest" },
        ],
        vec![doc! { "$match": {} }, doc! { "$out": "" }],
        vec![doc! { "$out": "secret_dest", "$match": {} }],
    ];
    for pipeline in &cases {
        let m = shape_message(pipeline);
        assert!(!m.contains("secret"), "{m}");
        assert!(!m.contains("hunter2"), "{m}");
    }
    assert!(shape_message(&cases[2]).contains("$out"));
    assert!(shape_message(&cases[4]).contains("$out"));
}

#[test]
fn a_target_in_a_server_database_is_refused_by_the_plan() {
    for db in ["admin", "local", "config"] {
        let out = format!(r#"{{"$out": {{"db": "{db}", "coll": "x"}}}}"#);
        assert_eq!(
            plan_target(&out),
            Err(PipelineError::ReservedTargetDatabase { index: 1 }),
            "{db}"
        );
        let merge = format!(r#"{{"$merge": {{"into": {{"db": "{db}", "coll": "x"}}}}}}"#);
        assert_eq!(
            plan_target(&merge),
            Err(PipelineError::ReservedTargetDatabase { index: 1 }),
            "{db}"
        );
    }
    assert_eq!(
        plan_target(r#"{"$out": {"db": "adminx", "coll": "x"}}"#),
        Ok(target(Some("adminx"), "x"))
    );
}

#[test]
fn a_target_in_a_server_database_is_refused_by_the_shape_check() {
    for db in ["admin", "local", "config"] {
        let pipeline = [doc! { "$out": { "db": db, "coll": "x" } }];
        assert_eq!(
            ensure_run_shape(&source(), &pipeline),
            Err(CoreError::ReservedDatabase),
            "{db}"
        );
    }
    let allowed = [doc! { "$out": { "db": "warehouse", "coll": "x" } }];
    assert!(ensure_run_shape(&source(), &allowed).is_ok());
}

#[test]
fn a_bare_target_follows_the_source_database_in_the_shape_check() {
    let bare = [doc! { "$match": {} }, doc! { "$out": "x" }];
    let merge = [doc! { "$merge": { "into": "x" } }];
    for db in ["admin", "local", "config"] {
        let from = crate::store::Namespace::new(db, "src");
        assert_eq!(
            ensure_run_shape(&from, &bare),
            Err(CoreError::ReservedDatabase),
            "{db}"
        );
        assert_eq!(
            ensure_run_shape(&from, &merge),
            Err(CoreError::ReservedDatabase),
            "{db}"
        );
        // A named, allowed database wins over the source's.
        let named = [doc! { "$out": { "db": "warehouse", "coll": "x" } }];
        assert!(ensure_run_shape(&from, &named).is_ok(), "{db}");
    }
    assert!(ensure_run_shape(&source(), &bare).is_ok());
}
