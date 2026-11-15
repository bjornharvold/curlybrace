//! What an explain would send, or why it cannot be sent: the enabled stages up to the selected
//! one (the preview's pipeline without its `$limit`, a trailing write stage dropped), by the same
//! rules the preview applies. Asked on every repaint (the Explain button's state and the "out of
//! date" mark), but answering means parsing every stage, so the answer is remembered on the
//! inputs it was computed from (`InputMemo`): a repaint with nothing changed parses nothing.
use super::aggregate::{AggregateState, Mode};
use super::aggregate_view::AggregateView;
use super::code_export::CodeBlock;
use super::input_memo::InputMemo;
use bson::Document;
use curlybrace_core::pipeline::{Stage, explain_pipeline, pipeline_text};
use std::sync::Arc;

/// The pipeline an explain sends, or why there is none (the preview's own reasons, plus the text
/// of text mode not applied yet).
pub type ExplainRequest = Result<Vec<Document>, CodeBlock>;

/// `stages` standing in for the draft's own (the text still being typed applied) as an explain
/// request: the text-mode buffer must not hold edits the stages do not have yet, since the
/// stages are what would be explained.
pub fn explain_request(draft: &AggregateState, stages: &[Stage]) -> ExplainRequest {
    let text_edited = draft.mode == Mode::Text && draft.text != pipeline_text(stages);
    if text_edited {
        return Err(CodeBlock::TextNotApplied);
    }
    explain_pipeline(stages, draft.selected).map_err(CodeBlock::Pipeline)
}

/// What the answer was computed from.
#[derive(Clone, Debug)]
pub struct ExplainKey {
    stages: Vec<Stage>,
    selected: usize,
    mode: Mode,
    text: Option<String>,
}

impl ExplainKey {
    fn is_of(&self, stages: &[Stage], selected: usize, mode: Mode, text: Option<&str>) -> bool {
        self.stages == stages
            && self.selected == selected
            && self.mode == mode
            && self.text.as_deref() == text
    }
}

/// Shared, so a hit copies a pointer rather than the pipeline.
pub type ExplainMemo = InputMemo<ExplainKey, Arc<ExplainRequest>>;

impl AggregateView {
    /// `explain_request`, remembered between repaints on the stages, selection, mode and (in
    /// text mode) the buffer.
    pub fn explain_request(&self, draft: &AggregateState, stages: &[Stage]) -> Arc<ExplainRequest> {
        let text = (draft.mode == Mode::Text).then_some(draft.text.as_str());
        self.explain_memo.answer(
            |key| key.is_of(stages, draft.selected, draft.mode, text),
            || ExplainKey {
                stages: stages.to_vec(),
                selected: draft.selected,
                mode: draft.mode,
                text: text.map(str::to_string),
            },
            |_| Arc::new(explain_request(draft, stages)),
        )
    }

    /// How many times `explain_request` really parsed (the seam that shows a repaint does not).
    pub fn explain_evaluations(&self) -> u32 {
        self.explain_memo.evaluations()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::pipeline::PipelineError;

    fn draft() -> AggregateState {
        AggregateState::default()
            .add_stage("$match")
            .add_stage("$sort")
            .add_stage("$out")
    }

    #[test]
    fn the_request_is_the_enabled_stages_up_to_the_selected_one_without_a_limit() {
        let d = draft().select(1);
        let request = explain_request(&d, &d.stages).expect("valid");
        assert_eq!(request.len(), 2);
        assert!(request.iter().all(|s| !s.contains_key("$limit")));
    }

    #[test]
    fn a_trailing_write_stage_is_dropped_so_the_stage_before_it_is_explainable() {
        let d = draft().select(2);
        let request = explain_request(&d, &d.stages).expect("valid");
        assert_eq!(request.len(), 2);
        assert!(request.iter().all(|s| !s.contains_key("$out")));
    }

    #[test]
    fn the_disabled_matrix_names_the_previews_reason() {
        let none = AggregateState::default();
        assert_eq!(
            explain_request(&none, &none.stages),
            Err(CodeBlock::Pipeline(PipelineError::Empty))
        );
        let off = draft().toggle(0).toggle(1).toggle(2);
        assert_eq!(
            explain_request(&off, &off.stages),
            Err(CodeBlock::Pipeline(PipelineError::Empty))
        );
        let broken = draft().edit(0, "{ not json").select(1);
        assert!(matches!(
            explain_request(&broken, &broken.stages),
            Err(CodeBlock::Pipeline(PipelineError::NotAStage { index: 0 }))
        ));
        let write_first = draft().edit(0, "{ \"$out\": \"x\" }").select(2);
        assert!(matches!(
            explain_request(&write_first, &write_first.stages),
            Err(CodeBlock::Pipeline(PipelineError::WriteStageNotLast { .. }))
        ));
        let nested = draft().edit(
            1,
            "{ \"$lookup\": { \"from\": \"a\", \"pipeline\": [ { \"$out\": \"x\" } ], \"as\": \"b\" } }",
        );
        assert!(matches!(
            explain_request(&nested, &nested.stages),
            Err(CodeBlock::Pipeline(PipelineError::NestedWriteStage { .. }))
        ));
    }

    #[test]
    fn a_write_stage_after_the_selected_stage_is_not_in_the_explained_range() {
        let d = draft().edit(2, "{ \"$out\": \"x\" }").select(0);
        let request = explain_request(&d, &d.stages).expect("valid");
        assert_eq!(request.len(), 1);
    }

    #[test]
    fn edited_text_that_is_not_applied_blocks_the_request() {
        let d = draft().select(1).to_text().with_text("[]");
        assert_eq!(
            explain_request(&d, &d.stages),
            Err(CodeBlock::TextNotApplied)
        );
        let unchanged = draft().select(1).to_text();
        assert!(explain_request(&unchanged, &unchanged.stages).is_ok());
    }

    #[test]
    fn a_repaint_with_nothing_changed_parses_nothing_and_a_change_asks_again() {
        let view = AggregateView::default();
        let d = draft().select(1);
        let first = view.explain_request(&d, &d.stages);
        let second = view.explain_request(&d, &d.stages);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(view.explain_evaluations(), 1);
        let moved = d.clone().select(0);
        view.explain_request(&moved, &moved.stages);
        assert_eq!(view.explain_evaluations(), 2, "another selection");
        let edited = moved.edit(0, "{ \"$match\": { \"a\": 1 } }");
        view.explain_request(&edited, &edited.stages);
        assert_eq!(view.explain_evaluations(), 3, "another stage text");
    }
}
