//! A repaint with nothing changed parses nothing; any change of an input asks again.
use super::*;
use crate::viewmodel::aggregate::Mode;
use curlybrace_core::pipeline::PipelineError;
use curlybrace_core::query::QueryText;

const MATCH: &str = r#"{ "$match": { "status": "active" } }"#;
const OUT: &str = r#"{ "$out": "totals" }"#;

fn draft(texts: &[&str]) -> AggregateState {
    texts.iter().fold(AggregateState::default(), |draft, text| {
        let at = draft.stages.len();
        draft.add_stage("$match").edit(at, text)
    })
}

#[test]
fn repaints_with_the_same_draft_parse_the_stages_once() {
    let view = AggregateView::default();
    let draft = draft(&[MATCH]);
    for _ in 0..5 {
        assert_eq!(view.code_block(&draft, &draft.stages), None);
    }
    assert_eq!(view.code_block_evaluations(), 1);
}

#[test]
fn a_changed_stage_toggle_text_or_mode_asks_again() {
    let view = AggregateView::default();
    let one = draft(&[MATCH]);
    assert_eq!(view.code_block(&one, &one.stages), None);
    let edited = one.clone().edit(0, "{ not json");
    assert_eq!(
        view.code_block(&edited, &edited.stages),
        Some(CodeBlock::Pipeline(PipelineError::NotAStage { index: 0 }))
    );
    assert_eq!(view.code_block_evaluations(), 2);
    let toggled = edited.toggle(0);
    assert_eq!(
        view.code_block(&toggled, &toggled.stages),
        Some(CodeBlock::Pipeline(PipelineError::Empty))
    );
    assert_eq!(view.code_block_evaluations(), 3);
    let text_mode = Mode::Text;
    let in_text = AggregateState {
        mode: text_mode,
        text: "[]".into(),
        ..one.clone()
    };
    assert_eq!(
        view.code_block(&in_text, &in_text.stages),
        Some(CodeBlock::TextNotApplied)
    );
    assert_eq!(view.code_block_evaluations(), 4);
    let retyped = AggregateState {
        text: "[{}]".into(),
        ..in_text
    };
    assert_eq!(
        view.code_block(&retyped, &retyped.stages),
        Some(CodeBlock::TextNotApplied)
    );
    assert_eq!(view.code_block_evaluations(), 5);
}

#[test]
fn the_text_being_typed_is_part_of_the_inputs() {
    let view = AggregateView::default();
    let draft = draft(&[MATCH]);
    let typed = vec![Stage {
        text: OUT.into(),
        ..draft.stages[0].clone()
    }];
    assert_eq!(view.code_block(&draft, &draft.stages), None);
    assert_eq!(view.code_block(&draft, &typed), None);
    assert_eq!(view.code_block(&draft, &typed), None);
    assert_eq!(view.code_block_evaluations(), 2);
    let broken = vec![Stage {
        text: "x".into(),
        ..typed[0].clone()
    }];
    assert!(view.code_block(&draft, &broken).is_some());
    assert_eq!(view.code_block_evaluations(), 3);
}

#[test]
fn the_query_text_is_parsed_once_per_change() {
    let state = FindBarState {
        text: QueryText {
            filter: r#"{ "a": 1 }"#.into(),
            ..QueryText::default()
        },
        ..FindBarState::default()
    };
    for _ in 0..5 {
        assert_eq!(state.export_error(), None);
    }
    assert_eq!(state.export_error_evaluations(), 1);
    let broken = FindBarState {
        text: QueryText {
            filter: "{".into(),
            ..state.text.clone()
        },
        ..state.clone()
    };
    assert!(broken.export_error().is_some());
    assert!(broken.export_error().is_some());
    assert_eq!(
        broken.export_error_evaluations(),
        2,
        "one more, for the change"
    );
    let fixed = FindBarState {
        text: state.text.clone(),
        ..broken
    };
    assert_eq!(fixed.export_error(), None);
    assert_eq!(fixed.export_error_evaluations(), 3);
}

#[test]
fn a_limit_is_part_of_the_query_inputs() {
    let state = FindBarState::default();
    assert_eq!(state.export_error(), None);
    let bad = FindBarState {
        text: QueryText {
            limit: "-1".into(),
            ..QueryText::default()
        },
        ..state
    };
    assert!(bad.export_error().is_some());
}
