//! The Aggregate draft's link to saved pipelines: which one was opened (or last saved), what it
//! looked like then, and the dialog that is open. Per session, kept through a lost connection
//! like the draft. Every op returns a new state; nothing here touches a file.
use super::aggregate::{AggregateState, Mode, PreviewState};
use crate::I18n;
use crate::app::pipelines_store::MAX_FILE_BYTES;
use curlybrace_core::pipeline::{Stage, parse_pipeline_text, pipeline_text};
use curlybrace_core::saved_pipeline::{
    MAX_NAME_CHARS, MAX_PER_CONNECTION, MAX_STAGE_BYTES, SavedError, SavedPipeline, SavedStage,
    name_taken_in, validate_name,
};
use std::cell::Cell;
use uuid::Uuid;

/// The dialog the Save / Open controls have open. The pipelines are named by id, so a dialog
/// never holds a stale copy of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavedDialog {
    /// Asks for a name; `replace_prompt` once that name was found taken and replacing it needs
    /// a second confirmation.
    Save {
        name: String,
        replace_prompt: bool,
    },
    /// The draft has unsaved changes; opening this pipeline would replace them.
    ConfirmOpen {
        id: Uuid,
    },
    Rename {
        id: Uuid,
        name: String,
    },
    ConfirmDelete {
        id: Uuid,
    },
}

/// What a session knows about saved pipelines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedUi {
    /// The pipeline the draft was opened from or last saved as: its id and name.
    pub opened: Option<(Uuid, String)>,
    /// The (text, enabled) list the draft is compared against for the "modified" mark.
    pub baseline: Option<Vec<SavedStage>>,
    pub dialog: Option<SavedDialog>,
    /// Why the last confirm of the open dialog failed, as an i18n key. Shown inline in the
    /// dialog and cleared whenever the dialog changes.
    pub error: Option<&'static str>,
    /// The answer of `is_modified` until the draft or the baseline changes.
    pub(super) modified: ModifiedMemo,
}

/// A remembered answer of `AggregateState::is_modified`, asked on every repaint. Whatever
/// changes the stages, the text-mode buffer or the baseline must reset it (see
/// `AggregateState::changed` and `with_text`); a dialog or an error never does. Two memos are
/// always equal: it says nothing about the draft that the draft does not already say.
#[derive(Clone, Debug, Default)]
pub struct ModifiedMemo {
    answer: Cell<Option<bool>>,
    evaluations: Cell<u32>,
}

impl PartialEq for ModifiedMemo {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for ModifiedMemo {}

impl SavedUi {
    /// The remembered "modified" answer is dropped: the draft or the baseline changed.
    pub(super) fn with_modified_forgotten(self) -> Self {
        Self {
            modified: ModifiedMemo::default(),
            ..self
        }
    }

    /// How many times the draft was really compared with its baseline (the seam that shows
    /// a repaint does not).
    pub fn modified_evaluations(&self) -> u32 {
        self.modified.evaluations.get()
    }

    /// `dialog` (or none) is shown, with no error from an earlier attempt.
    pub fn with_dialog(self, dialog: Option<SavedDialog>) -> Self {
        Self {
            dialog,
            error: None,
            ..self
        }
    }

    /// The open dialog stays and shows why its confirm failed.
    pub fn with_error(self, key: &'static str) -> Self {
        Self {
            error: Some(key),
            ..self
        }
    }
}

/// The i18n key of the message for a refused change to the list.
pub fn saved_error_key(error: SavedError) -> &'static str {
    match error {
        SavedError::EmptyName => "aggregate.saved.error.empty_name",
        SavedError::InvalidName => "aggregate.saved.error.invalid_name",
        SavedError::NameTooLong => "aggregate.saved.error.name_too_long",
        SavedError::NameTaken => "aggregate.saved.error.name_taken",
        SavedError::TooMany => "aggregate.saved.error.too_many",
        SavedError::TooLarge => "aggregate.saved.error.too_large",
        SavedError::Unsupported => "aggregate.saved.error.unsupported",
    }
}

/// `bytes` as whole megabytes, rounded: 1 000 000 and 5 MiB read as 1 and 5.
fn megabytes(bytes: usize) -> String {
    format!("{:.0}", bytes as f64 / (1024.0 * 1024.0))
}

/// The text of an error key of the saved pipelines. A message that names a limit has it filled
/// in from the constant that enforces it (`{max}`), so the text can never disagree with the rule.
pub fn saved_message(i18n: &I18n, key: &str) -> String {
    let max = match key {
        "aggregate.saved.error.too_many" => MAX_PER_CONNECTION.to_string(),
        "aggregate.saved.error.name_too_long" => MAX_NAME_CHARS.to_string(),
        "aggregate.saved.error.too_large" => megabytes(MAX_STAGE_BYTES),
        "aggregate.saved.error.file_too_large" => megabytes(MAX_FILE_BYTES),
        _ => return i18n.t(key),
    };
    i18n.t_with(key, &[("max", &max)])
}

/// Why `name` cannot be used, as an error: empty or too long, or (when `check_taken`) used by
/// another of `entries` than `except`.
pub fn name_problem(
    entries: &[SavedPipeline],
    name: &str,
    except: Option<Uuid>,
    check_taken: bool,
) -> Option<SavedError> {
    if let Err(e) = validate_name(name) {
        return Some(e);
    }
    (check_taken && name_taken_in(entries, name, except)).then_some(SavedError::NameTaken)
}

fn to_saved(stages: &[Stage]) -> Vec<SavedStage> {
    stages
        .iter()
        .map(|s| SavedStage {
            text: s.text.clone(),
            enabled: s.enabled,
        })
        .collect()
}

fn from_saved(stages: &[SavedStage]) -> Vec<Stage> {
    stages
        .iter()
        .map(|s| Stage {
            text: s.text.clone(),
            enabled: s.enabled,
        })
        .collect()
}

impl AggregateState {
    /// Whether text mode holds edits that do not parse (so they cannot be saved or compared).
    pub fn text_is_invalid(&self) -> bool {
        self.mode == Mode::Text
            && self.text != pipeline_text(&self.stages)
            && parse_pipeline_text(&self.text).is_err()
    }

    /// The pipeline as it would be saved: the stages, or in text mode the edited buffer when
    /// it parses (every stage enabled, as `from_text` does). Unparsable text gives the stages.
    pub fn as_saved_stages(&self) -> Vec<SavedStage> {
        let edited = self.mode == Mode::Text && self.text != pipeline_text(&self.stages);
        match edited.then(|| parse_pipeline_text(&self.text)) {
            Some(Ok(parsed)) => to_saved(&parsed),
            _ => to_saved(&self.stages),
        }
    }

    /// Whether the draft differs from what was opened or saved, comparing text and enabled
    /// flag per stage and ignoring stage keys. A draft that was never saved or opened is
    /// modified exactly when it has stages: an empty draft has nothing to lose. Unparsable
    /// text-mode edits count as modified.
    pub fn is_modified(&self) -> bool {
        let memo = &self.saved.modified;
        if let Some(answer) = memo.answer.get() {
            return answer;
        }
        let answer = self.modified_uncached();
        memo.answer.set(Some(answer));
        memo.evaluations.set(memo.evaluations.get().wrapping_add(1));
        answer
    }

    /// The comparison itself, for `is_modified` to remember.
    pub(super) fn modified_uncached(&self) -> bool {
        if self.text_is_invalid() {
            return true;
        }
        let current = self.as_saved_stages();
        match &self.saved.baseline {
            Some(baseline) => *baseline != current,
            None => !current.is_empty(),
        }
    }

    /// The draft replaced by `saved`: new keys (never reused), the last stage selected, stage
    /// mode, and the pane idle so the tab's auto refresh asks again. The preview request is
    /// bumped, so an answer still in flight is stale; its slot stays taken until it arrives,
    /// as for any change. Nothing is sent here.
    pub fn opened_from(self, saved: &SavedPipeline) -> Self {
        let stages = from_saved(&saved.stages);
        let (keys, next_key) = self.keyed(Vec::new(), stages.len());
        let selected = Self::clamp(usize::MAX, stages.len());
        Self {
            stages,
            keys,
            next_key,
            selected,
            mode: Mode::Stages,
            text: String::new(),
            text_error: None,
            preview: PreviewState::Idle,
            saved: SavedUi {
                opened: Some((saved.id, saved.name.clone())),
                baseline: Some(saved.stages.clone()),
                ..SavedUi::default()
            },
            ..self
        }
        .on_collection_changed()
    }

    /// The draft was saved as `name` (entry `id`): it is now that pipeline, unmodified.
    pub fn saved_as(self, id: Uuid, name: &str) -> Self {
        let baseline = Some(self.as_saved_stages());
        Self {
            saved: SavedUi {
                opened: Some((id, name.to_string())),
                baseline,
                ..SavedUi::default()
            },
            ..self
        }
    }

    /// The opened pipeline was renamed: only its shown name changes.
    pub fn with_opened_name(self, name: &str) -> Self {
        let opened = self
            .saved
            .opened
            .as_ref()
            .map(|(id, _)| (*id, name.to_string()));
        Self {
            saved: SavedUi {
                opened,
                ..self.saved
            },
            ..self
        }
    }

    /// The opened pipeline is gone: the draft stays, as a draft nobody saved.
    pub fn without_opened(self) -> Self {
        Self {
            saved: SavedUi {
                opened: None,
                baseline: None,
                ..self.saved
            }
            .with_modified_forgotten(),
            ..self
        }
    }

    pub fn with_saved_dialog(self, dialog: Option<SavedDialog>) -> Self {
        Self {
            saved: self.saved.with_dialog(dialog),
            ..self
        }
    }

    pub fn with_saved_error(self, key: &'static str) -> Self {
        Self {
            saved: self.saved.with_error(key),
            ..self
        }
    }
}

#[cfg(test)]
#[path = "aggregate_saved_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "aggregate_saved_memo_tests.rs"]
mod memo_tests;
