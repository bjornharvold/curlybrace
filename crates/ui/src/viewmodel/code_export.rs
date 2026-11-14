//! The Copy as code dialog's state: the request it was opened for, the language and options, and
//! the code generated from them. The text is regenerated on every change (pure and cheap), so
//! what the preview shows and what Copy puts on the clipboard are the same string. The language
//! and options last used are remembered for the rest of the app run (never saved to disk). Nothing
//! here is sent anywhere, and the generated code is never logged (not even by `Debug`).
use super::aggregate::{AggregateState, Mode};
use bson::{Bson, Document};
use curlybrace_core::codegen::{CodeOptions, CodeRequest, CodegenError, Language, generate};
use curlybrace_core::pipeline::{PREVIEW_LIMIT, PipelineError, pipeline_text, preview_pipeline};
use std::fmt;

/// Which variable-name field of the dialog is meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameField {
    Client,
    Db,
    Collection,
}

/// Why the Aggregate tab cannot offer its code.
#[derive(Clone, Debug, PartialEq)]
pub enum CodeBlock {
    /// A stage does not parse, a write stage is misplaced, or no stage is enabled.
    Pipeline(PipelineError),
    /// The text-mode buffer was edited and not applied to the stages yet.
    TextNotApplied,
}

/// The enabled stages of the draft exactly as a run would send them: the preview's `$limit` is
/// never part of them, and a trailing `$out` / `$merge` is kept (the dialog's option drops it).
pub fn aggregate_stages(draft: &AggregateState) -> Result<Vec<Document>, CodeBlock> {
    let text_edited = draft.mode == Mode::Text && draft.text != pipeline_text(&draft.stages);
    if text_edited {
        return Err(CodeBlock::TextNotApplied);
    }
    let plan =
        preview_pipeline(&draft.stages, usize::MAX, PREVIEW_LIMIT).map_err(CodeBlock::Pipeline)?;
    // The plan ends in the preview's `$limit`; a trailing write stage is not in it at all.
    let mut stages = plan.pipeline;
    stages.pop();
    if plan.not_run.is_empty() {
        return Ok(stages);
    }
    let index = draft.stages.iter().rposition(|stage| stage.enabled);
    let write = index
        .and_then(|i| serde_json::from_str::<serde_json::Value>(&draft.stages[i].text).ok())
        .and_then(|value| Bson::try_from(value).ok())
        .and_then(|bson| match bson {
            Bson::Document(doc) => Some(doc),
            _ => None,
        })
        .ok_or(CodeBlock::Pipeline(PipelineError::NotAStage {
            index: index.unwrap_or(0),
        }))?;
    stages.push(write);
    Ok(stages)
}

/// An open dialog: what it shows code for, and the code.
#[derive(Clone, PartialEq)]
pub struct CodeDialog {
    pub request: CodeRequest,
    pub language: Language,
    pub options: CodeOptions,
    /// The generated code, or why there is none (shown in its place).
    pub text: Result<String, CodegenError>,
}

impl fmt::Debug for CodeDialog {
    /// Names the language and whether code exists; never the request or the code.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeDialog")
            .field("language", &self.language)
            .field("generated", &self.text.is_ok())
            .finish_non_exhaustive()
    }
}

impl CodeDialog {
    fn new(request: CodeRequest, language: Language, options: CodeOptions) -> Self {
        let text = generate(language, &request, &options);
        Self {
            request,
            language,
            options,
            text,
        }
    }

    /// Whether the request is an aggregation (only it has a write stage to include or not).
    pub fn is_aggregate(&self) -> bool {
        matches!(self.request, CodeRequest::Aggregate { .. })
    }

    /// The text Copy puts on the clipboard: the previewed code, none while there is an error.
    pub fn clipboard_text(&self) -> Option<&str> {
        self.text.as_deref().ok()
    }

    fn with_language(self, language: Language) -> Self {
        Self::new(self.request, language, self.options)
    }

    fn with_options(self, options: CodeOptions) -> Self {
        Self::new(self.request, self.language, options)
    }
}

/// The dialog (when open) and the language and options it starts with next time.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeExportUi {
    pub dialog: Option<CodeDialog>,
    /// What the last dialog ended with; names that were not valid are not kept.
    pub last: (Language, CodeOptions),
}

impl Default for CodeExportUi {
    fn default() -> Self {
        Self {
            dialog: None,
            last: (Language::Mongosh, CodeOptions::default()),
        }
    }
}

impl CodeExportUi {
    /// The dialog opened for `request` with the remembered language and options.
    pub fn opened(self, request: CodeRequest) -> Self {
        let (language, options) = self.last.clone();
        Self {
            dialog: Some(CodeDialog::new(request, language, options)),
            ..self
        }
    }

    pub fn closed(self) -> Self {
        Self {
            dialog: None,
            ..self
        }
    }

    pub fn language_set(self, language: Language) -> Self {
        self.edited(|dialog| dialog.with_language(language))
    }

    pub fn setup_set(self, setup: bool) -> Self {
        self.options_edited(|options| CodeOptions { setup, ..options })
    }

    pub fn write_stage_set(self, write_stage: bool) -> Self {
        self.options_edited(|options| CodeOptions {
            write_stage,
            ..options
        })
    }

    pub fn name_set(self, field: NameField, name: String) -> Self {
        self.options_edited(|options| match field {
            NameField::Client => CodeOptions {
                client_var: name,
                ..options
            },
            NameField::Db => CodeOptions {
                db_var: name,
                ..options
            },
            NameField::Collection => CodeOptions {
                collection_var: name,
                ..options
            },
        })
    }

    fn options_edited(self, change: impl FnOnce(CodeOptions) -> CodeOptions) -> Self {
        self.edited(|dialog| {
            let options = change(dialog.options.clone());
            dialog.with_options(options)
        })
    }

    /// The dialog changed by `change`, and what it ends with remembered. Nothing when closed.
    fn edited(self, change: impl FnOnce(CodeDialog) -> CodeDialog) -> Self {
        let Self { dialog, last } = self;
        let Some(dialog) = dialog else {
            return Self { dialog, last };
        };
        let dialog = change(dialog);
        // Names that are not usable are not carried to the next dialog.
        let options = dialog.options.clone().validated().unwrap_or(CodeOptions {
            setup: dialog.options.setup,
            write_stage: dialog.options.write_stage,
            ..last.1
        });
        Self {
            last: (dialog.language, options),
            dialog: Some(dialog),
        }
    }

    /// The text Copy puts on the clipboard, if a dialog is open and has code.
    pub fn clipboard_text(&self) -> Option<&str> {
        self.dialog.as_ref().and_then(CodeDialog::clipboard_text)
    }
}

#[cfg(test)]
#[path = "code_export_tests.rs"]
mod tests;
