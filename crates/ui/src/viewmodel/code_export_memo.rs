//! Whether the two Copy as code buttons are on is asked on every repaint, but answering means
//! parsing every stage (or the query bar text). The answers are remembered on the inputs they
//! were computed from (`InputMemo`), so a repaint with nothing changed parses nothing, and any
//! change of the stages, the mode, the buffer or the query text asks again.
use super::aggregate::{AggregateState, Mode};
use super::aggregate_view::AggregateView;
use super::code_export::{CodeBlock, stages_exported};
use super::findbar::FindBarState;
use super::input_memo::InputMemo;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::Stage;
use curlybrace_core::query::parse_export_query;

/// What the Aggregate button's answer was computed from: the stages (the draft's own, or with
/// the text still being typed applied), the mode, and the buffer only in text mode (stage mode
/// does not read it).
#[derive(Clone, Debug)]
pub struct CodeKey {
    stages: Vec<Stage>,
    mode: Mode,
    text: Option<String>,
    /// The options form has a problem (the answer is then `CodeBlock::Options`).
    blocked: bool,
}

impl CodeKey {
    fn is_of(&self, stages: &[Stage], mode: Mode, text: Option<&str>, blocked: bool) -> bool {
        self.stages == stages
            && self.mode == mode
            && self.text.as_deref() == text
            && self.blocked == blocked
    }
}

pub type CodeBlockMemo = InputMemo<CodeKey, Option<CodeBlock>>;
pub type FindExportMemo = InputMemo<curlybrace_core::query::QueryText, Option<CoreError>>;

impl AggregateView {
    /// Why the Aggregate tab has no code to offer for `stages` (standing in for the draft's own),
    /// or `None` when it has; the same answer as `stages_exported`, remembered between repaints.
    pub fn code_block(&self, draft: &AggregateState, stages: &[Stage]) -> Option<CodeBlock> {
        let text = (draft.mode == Mode::Text).then_some(draft.text.as_str());
        let blocked = draft.options_blocked();
        self.code_block.answer(
            |key| key.is_of(stages, draft.mode, text, blocked),
            || CodeKey {
                stages: stages.to_vec(),
                mode: draft.mode,
                text: text.map(str::to_string),
                blocked,
            },
            |_| stages_exported(draft, stages).err(),
        )
    }

    /// How many times `code_block` really parsed (the seam that shows a repaint does not).
    pub fn code_block_evaluations(&self) -> u32 {
        self.code_block.evaluations()
    }
}

impl FindBarState {
    /// Why the query bar has no code to offer (its text does not parse as typed), or `None`;
    /// remembered between repaints on the text.
    pub fn export_error(&self) -> Option<CoreError> {
        self.export_memo.answer(
            |text| *text == self.text,
            || self.text.clone(),
            |text| parse_export_query(text).err(),
        )
    }

    /// How many times `export_error` really parsed.
    pub fn export_error_evaluations(&self) -> u32 {
        self.export_memo.evaluations()
    }
}

#[cfg(test)]
#[path = "code_export_memo_tests.rs"]
mod tests;
