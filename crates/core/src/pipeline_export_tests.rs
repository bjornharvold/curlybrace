//! The stages Copy as code exports: the enabled stages as a run would send them, no preview
//! limit, a write stage kept (alone or last) and the shape refused as a run refuses it.
use super::*;
use bson::doc;

fn on(text: &str) -> Stage {
    Stage {
        text: text.to_string(),
        enabled: true,
    }
}

fn off(text: &str) -> Stage {
    Stage {
        enabled: false,
        ..on(text)
    }
}

#[test]
fn the_enabled_stages_are_exported_without_a_limit() {
    let stages = [on(r#"{"$match":{"a":1}}"#), off(r#"{"$sort":{"a":1}}"#)];
    assert_eq!(
        export_stages(&stages),
        Ok(vec![doc! { "$match": { "a": 1 } }])
    );
}

#[test]
fn a_lone_write_stage_is_exported() {
    assert_eq!(
        export_stages(&[on(r#"{"$out":"totals"}"#)]),
        Ok(vec![doc! { "$out": "totals" }])
    );
}

#[test]
fn a_trailing_write_stage_is_kept() {
    let stages = [on(r#"{"$match":{}}"#), on(r#"{"$merge":{"into":"t"}}"#)];
    let exported = export_stages(&stages).unwrap();
    assert_eq!(exported.last(), Some(&doc! { "$merge": { "into": "t" } }));
    assert_eq!(exported.len(), 2);
}

#[test]
fn nothing_enabled_is_empty() {
    assert_eq!(export_stages(&[]), Err(PipelineError::Empty));
    assert_eq!(
        export_stages(&[off(r#"{"$match":{}}"#)]),
        Err(PipelineError::Empty)
    );
}

#[test]
fn an_unparsable_stage_is_refused_with_its_position() {
    let stages = [on(r#"{"$match":{}}"#), on("{ nope")];
    assert_eq!(
        export_stages(&stages),
        Err(PipelineError::NotAStage { index: 1 })
    );
}

#[test]
fn a_misplaced_or_nested_write_stage_is_refused() {
    let early = [on(r#"{"$out":"t"}"#), on(r#"{"$match":{}}"#)];
    assert!(matches!(
        export_stages(&early),
        Err(PipelineError::WriteStageNotLast { index: 0, .. })
    ));
    let nested = [
        on(r#"{"$facet":{"x":[{"$out":"t"}]}}"#),
        on(r#"{"$limit":1}"#),
    ];
    assert!(matches!(
        export_stages(&nested),
        Err(PipelineError::NestedWriteStage { index: 0, .. })
    ));
}
