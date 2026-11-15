//! The preview, the export and the run share one check of where write stages may be: these pin
//! that the three agree on the same inputs (each keeps its own policy for what it accepts).
use super::*;

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

const MATCH: &str = r#"{"$match":{"a":1}}"#;
const OUT: &str = r#"{"$out":"totals"}"#;
const MERGE: &str = r#"{"$merge":{"into":"totals"}}"#;
const FACET_WRITE: &str = r#"{"$facet":{"x":[{"$out":"t"}]}}"#;
const LOOKUP_WRITE: &str =
    r#"{"$lookup":{"from":"c","as":"r","pipeline":[{"$merge":{"into":"t"}}]}}"#;

/// What each of the three says about `stages`, as the error alone (`None` when it accepts).
fn verdicts(stages: &[Stage]) -> [Option<PipelineError>; 3] {
    [
        preview_pipeline(stages, usize::MAX - 1, 20).err(),
        export_stages(stages).err(),
        run_pipeline(stages).err(),
    ]
}

fn texts(list: &[&str]) -> Vec<Stage> {
    list.iter().map(|t| on(t)).collect()
}

#[test]
fn a_write_stage_last_is_accepted_by_all_three() {
    for write in [OUT, MERGE] {
        assert_eq!(verdicts(&texts(&[MATCH, write])), [None, None, None]);
    }
}

#[test]
fn a_write_stage_before_the_last_is_refused_by_all_three_at_the_same_stage() {
    for stages in [
        texts(&[OUT, MATCH]),
        texts(&[MATCH, MERGE, MATCH]),
        texts(&[MATCH, OUT, OUT]),
    ] {
        let [preview, export, run] = verdicts(&stages);
        assert!(
            matches!(preview, Some(PipelineError::WriteStageNotLast { .. })),
            "{preview:?}"
        );
        assert_eq!(preview, export);
        if stages.last().is_some_and(|s| s.text == OUT) {
            assert_eq!(preview, run);
        }
    }
}

#[test]
fn a_write_stage_nested_in_an_earlier_stage_is_refused_by_all_three() {
    for (nested, name) in [(FACET_WRITE, "$out"), (LOOKUP_WRITE, "$merge")] {
        let expected = Some(PipelineError::NestedWriteStage {
            index: 1,
            name: name.into(),
        });
        let stages = texts(&[MATCH, nested, OUT]);
        assert_eq!(
            verdicts(&stages),
            [expected.clone(), expected.clone(), expected]
        );
    }
}

#[test]
fn a_write_stage_nested_in_the_last_stage_is_refused_by_the_preview_and_the_export() {
    let [preview, export, run] = verdicts(&texts(&[MATCH, FACET_WRITE]));
    let expected = Some(PipelineError::NestedWriteStage {
        index: 1,
        name: "$out".into(),
    });
    assert_eq!(preview, expected);
    assert_eq!(export, expected);
    // A run needs a write stage last; that is the first thing it says is missing.
    assert_eq!(run, Some(PipelineError::NoWriteStage));
}

#[test]
fn several_write_stages_are_refused_by_all_three() {
    let [preview, export, run] = verdicts(&texts(&[OUT, MERGE, OUT]));
    let expected = Some(PipelineError::WriteStageNotLast {
        index: 0,
        name: "$out".into(),
    });
    assert_eq!(
        [preview, export, run],
        [expected.clone(), expected.clone(), expected]
    );
}

#[test]
fn no_enabled_stage_is_empty_for_all_three() {
    for stages in [vec![], vec![off(MATCH), off(OUT)]] {
        let expected = Some(PipelineError::Empty);
        assert_eq!(
            verdicts(&stages),
            [expected.clone(), expected.clone(), expected]
        );
    }
}

#[test]
fn positions_are_those_of_the_whole_list_with_disabled_stages_counted() {
    let stages = vec![off(MATCH), on(OUT), on(MATCH), on(OUT)];
    let expected = Some(PipelineError::WriteStageNotLast {
        index: 1,
        name: "$out".into(),
    });
    assert_eq!(
        verdicts(&stages),
        [expected.clone(), expected.clone(), expected]
    );
}

#[test]
fn a_lone_write_stage_is_what_each_one_makes_of_it() {
    let [preview, export, run] = verdicts(&texts(&[OUT]));
    assert_eq!(
        preview,
        Some(PipelineError::Empty),
        "nothing left to preview"
    );
    assert_eq!(export, None, "exported as it is");
    assert_eq!(run, None);
}

#[test]
fn only_the_run_requires_a_write_stage() {
    let [preview, export, run] = verdicts(&texts(&[MATCH]));
    assert_eq!((preview, export), (None, None));
    assert_eq!(run, Some(PipelineError::NoWriteStage));
}
