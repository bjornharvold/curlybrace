//! The Aggregate tab's draft: the pipeline stages, which one is previewed, the text-mode buffer
//! and the last preview. Every op returns a new state; nothing here talks to a server.
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{
    PREVIEW_LIMIT, PipelineError, Stage, parse_pipeline_text, pipeline_text, preview_pipeline,
};

/// The names the Add stage menu offers, in menu order.
pub const STAGE_MENU: [&str; 13] = [
    "$match",
    "$group",
    "$project",
    "$sort",
    "$limit",
    "$skip",
    "$unwind",
    "$lookup",
    "$addFields",
    "$count",
    "$facet",
    "$out",
    "$merge",
];

/// The template body of a new `name` stage (JSON data, not a label); an unknown name gets `{}`.
fn template_body(name: &str) -> &'static str {
    match name {
        "$group" => r#"{ "_id": null }"#,
        "$project" => r#"{ "_id": 1 }"#,
        "$sort" => r#"{ "_id": 1 }"#,
        "$limit" => "10",
        "$skip" => "0",
        "$unwind" => r#"{ "path": "$field" }"#,
        "$lookup" => {
            r#"{ "from": "collection", "localField": "field", "foreignField": "field", "as": "joined" }"#
        }
        "$count" => r#""count""#,
        "$out" => r#""collection""#,
        "$merge" => r#"{ "into": "collection" }"#,
        _ => "{}",
    }
}

/// The text of a new `name` stage, e.g. `{ "$match": {} }` laid out on three lines.
pub fn stage_template(name: &str) -> String {
    let key = serde_json::Value::String(name.to_string());
    format!("{{\n  {key}: {}\n}}", template_body(name))
}

/// Whether the pipeline is edited as a list of stage cards or as one JSON array.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Stages,
    Text,
}

/// The documents after the previewed stage, and the trailing write stages that were not run.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub docs: Vec<Document>,
    pub not_run: Vec<String>,
}

/// Why a preview is not shown: the pipeline was refused before sending, or the server failed.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewError {
    Pipeline(PipelineError),
    Core(CoreError),
}

/// The preview pane. `last` is the latest good preview, kept through a reload or a failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PreviewState {
    #[default]
    Idle,
    /// Request `AggregateState::request` is in flight.
    Loading {
        not_run: Vec<String>,
        last: Option<Preview>,
    },
    Shown(Preview),
    Failed {
        error: PreviewError,
        last: Option<Preview>,
    },
}

impl PreviewState {
    /// The latest good preview this state shows or keeps.
    pub fn into_last(self) -> Option<Preview> {
        match self {
            Self::Idle => None,
            Self::Shown(p) => Some(p),
            Self::Loading { last, .. } | Self::Failed { last, .. } => last,
        }
    }
}

/// One session's Aggregate draft. Nothing here is persisted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AggregateState {
    pub stages: Vec<Stage>,
    /// The stage whose output is previewed; `0` while there are none.
    pub selected: usize,
    pub mode: Mode,
    /// The text-mode buffer.
    pub text: String,
    /// Why the text-mode buffer cannot go back to stages (syntax errors carry line and column).
    pub text_error: Option<PipelineError>,
    pub preview: PreviewState,
    /// The newest preview request; bumped by every pipeline change so an older answer is stale.
    pub request: u64,
}

impl AggregateState {
    /// The highest valid selection for `len` stages.
    fn clamp(i: usize, len: usize) -> usize {
        i.min(len.saturating_sub(1))
    }

    /// The pipeline changed: an answer still in flight no longer matches it.
    fn changed(self) -> Self {
        Self {
            request: self.request.wrapping_add(1),
            ..self
        }
    }

    /// `self` with stage `i` replaced by `f(stage)`; out of range changes nothing.
    fn map_stage(self, i: usize, f: impl FnOnce(&Stage) -> Stage) -> Self {
        let Some(old) = self.stages.get(i) else {
            return self;
        };
        let new = f(old);
        if &new == old {
            return self;
        }
        let stages = self
            .stages
            .iter()
            .enumerate()
            .map(|(j, s)| if j == i { new.clone() } else { s.clone() })
            .collect();
        Self { stages, ..self }.changed()
    }

    /// `self` with stages `i` and `i + 1` swapped; the selection stays on the stage it was on.
    fn swapped(self, i: usize) -> Self {
        if i >= self.stages.len().saturating_sub(1) {
            return self;
        }
        let mut stages = self.stages.clone();
        stages.swap(i, i + 1);
        let selected = match self.selected {
            s if s == i => i + 1,
            s if s == i + 1 => i,
            s => s,
        };
        Self {
            stages,
            selected,
            ..self
        }
        .changed()
    }

    /// Appends a `name` stage with its template body and selects it.
    pub fn add_stage(self, name: &str) -> Self {
        let stages: Vec<Stage> = self
            .stages
            .iter()
            .cloned()
            .chain(std::iter::once(Stage {
                text: stage_template(name),
                enabled: true,
            }))
            .collect();
        let selected = stages.len() - 1;
        Self {
            stages,
            selected,
            ..self
        }
        .changed()
    }

    /// Drops stage `i`; the selection stays on the same stage, or the nearest one if it was `i`.
    pub fn remove(self, i: usize) -> Self {
        if i >= self.stages.len() {
            return self;
        }
        let stages: Vec<Stage> = self
            .stages
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, s)| s.clone())
            .collect();
        let selected = match self.selected > i {
            true => self.selected - 1,
            false => self.selected,
        };
        let selected = Self::clamp(selected, stages.len());
        Self {
            stages,
            selected,
            ..self
        }
        .changed()
    }

    /// Moves stage `i` one place up; the first stage stays.
    pub fn move_up(self, i: usize) -> Self {
        match i {
            0 => self,
            _ => self.swapped(i - 1),
        }
    }

    /// Moves stage `i` one place down; the last stage stays.
    pub fn move_down(self, i: usize) -> Self {
        self.swapped(i)
    }

    /// Enables or disables stage `i`; the selection is unchanged (a disabled stage can be shown).
    pub fn toggle(self, i: usize) -> Self {
        self.map_stage(i, |s| Stage {
            enabled: !s.enabled,
            ..s.clone()
        })
    }

    /// Replaces the text of stage `i`.
    pub fn edit(self, i: usize, text: &str) -> Self {
        self.map_stage(i, |s| Stage {
            text: text.to_string(),
            ..s.clone()
        })
    }

    /// Selects stage `i`, clamped to the last stage.
    pub fn select(self, i: usize) -> Self {
        let selected = Self::clamp(i, self.stages.len());
        if selected == self.selected {
            return self;
        }
        Self { selected, ..self }.changed()
    }

    /// Switches to text mode with the stages as one JSON array.
    pub fn to_text(self) -> Self {
        let text = pipeline_text(&self.stages);
        Self {
            mode: Mode::Text,
            text,
            text_error: None,
            ..self
        }
    }

    /// Replaces the text-mode buffer; an earlier error no longer applies.
    pub fn with_text(self, text: &str) -> Self {
        Self {
            text: text.to_string(),
            text_error: None,
            ..self
        }
    }

    /// Leaves text mode with the parsed stages, selecting the last; invalid text keeps
    /// everything as it was and records why.
    pub fn from_text(self) -> Self {
        match parse_pipeline_text(&self.text) {
            Ok(stages) => {
                let selected = Self::clamp(usize::MAX, stages.len());
                Self {
                    stages,
                    selected,
                    mode: Mode::Stages,
                    text_error: None,
                    ..self
                }
                .changed()
            }
            Err(error) => Self {
                text_error: Some(error),
                ..self
            },
        }
    }

    /// The pipeline now runs against another collection (or a new connection): an answer still
    /// in flight is stale and a shown preview no longer applies, so the pane is idle again. The
    /// draft itself stays.
    pub fn on_collection_changed(self) -> Self {
        Self {
            preview: PreviewState::Idle,
            ..self
        }
        .changed()
    }

    /// Whether the tab should ask for a preview on its own: the pane is idle with stages to run
    /// and request `sent` (the last one asked for automatically) is not the current one, so an
    /// ask that could not run (no connection) is not repeated every frame.
    pub fn wants_auto_refresh(&self, sent: Option<u64>) -> bool {
        self.preview == PreviewState::Idle && !self.stages.is_empty() && sent != Some(self.request)
    }

    /// Starts a preview of the selected stage: the numbered request and the read-only pipeline
    /// to send, or `None` when the pipeline is refused (shown as `Failed`, nothing sent).
    pub fn begin_preview(self) -> (Self, Option<(u64, Vec<Document>)>) {
        let request = self.request.wrapping_add(1);
        let plan = preview_pipeline(&self.stages, self.selected, PREVIEW_LIMIT);
        let last = self.preview.clone().into_last();
        match plan {
            Ok(plan) => {
                let preview = PreviewState::Loading {
                    not_run: plan.not_run,
                    last,
                };
                let next = Self {
                    preview,
                    request,
                    ..self
                };
                (next, Some((request, plan.pipeline)))
            }
            Err(error) => {
                let preview = PreviewState::Failed {
                    error: PreviewError::Pipeline(error),
                    last,
                };
                let next = Self {
                    preview,
                    request,
                    ..self
                };
                (next, None)
            }
        }
    }

    /// The answer to preview `request`; an answer to any other request, or one already
    /// applied, is stale and changes nothing. A failure keeps the last good preview.
    pub fn arrived(self, request: u64, outcome: Result<Vec<Document>, CoreError>) -> Self {
        if request != self.request {
            return self;
        }
        let PreviewState::Loading { not_run, last } = &self.preview else {
            return self;
        };
        let preview = match outcome {
            Ok(docs) => PreviewState::Shown(Preview {
                docs,
                not_run: not_run.clone(),
            }),
            Err(error) => PreviewState::Failed {
                error: PreviewError::Core(error),
                last: last.clone(),
            },
        };
        Self { preview, ..self }
    }
}

#[cfg(test)]
#[path = "aggregate_tests.rs"]
mod tests;
