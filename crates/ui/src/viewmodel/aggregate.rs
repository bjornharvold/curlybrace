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
    /// One identity per stage, in step with `stages`: assigned when a stage is created or loaded,
    /// kept when it moves, never reused, so the view can tie its focus, cursor, undo and
    /// completion list to the stage rather than to its position.
    pub keys: Vec<u64>,
    /// The identity the next new stage gets.
    pub(super) next_key: u64,
    /// The stage whose output is previewed; `0` while there are none.
    pub selected: usize,
    pub mode: Mode,
    /// The text-mode buffer.
    pub text: String,
    /// Why the text-mode buffer cannot go back to stages (syntax errors carry line and column).
    pub text_error: Option<PipelineError>,
    pub preview: PreviewState,
    /// The preview request sent and not answered yet: at most one per session (see
    /// `aggregate_flight`).
    pub in_flight: Option<u64>,
    /// The newest wanted preview while one is in flight.
    pub queued: Option<super::aggregate_flight::Queued>,
    /// The newest preview request; bumped by every pipeline change so an older answer is stale.
    pub request: u64,
}

impl AggregateState {
    /// The highest valid selection for `len` stages.
    fn clamp(i: usize, len: usize) -> usize {
        i.min(len.saturating_sub(1))
    }

    /// `stages` with fresh identities, appended to `kept` (the identities that stay).
    fn keyed(&self, kept: Vec<u64>, added: usize) -> (Vec<u64>, u64) {
        let first = self.next_key;
        let end = first.wrapping_add(added as u64);
        (kept.into_iter().chain(first..end).collect(), end)
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
        let mut keys = self.keys.clone();
        keys.swap(i, i + 1);
        let selected = match self.selected {
            s if s == i => i + 1,
            s if s == i + 1 => i,
            s => s,
        };
        Self {
            stages,
            keys,
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
        let (keys, next_key) = self.keyed(self.keys.clone(), 1);
        Self {
            stages,
            keys,
            next_key,
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
        let keys = self
            .keys
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, k)| *k)
            .collect();
        let selected = match self.selected > i {
            true => self.selected - 1,
            false => self.selected,
        };
        let selected = Self::clamp(selected, stages.len());
        Self {
            stages,
            keys,
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

    /// Leaves text mode. Text still as `to_text` wrote it goes back to the same stages, their
    /// enabled flags and the selection kept, and changes nothing. Edited text becomes the parsed
    /// stages (all enabled: the text has no place for the flag), selecting the last; invalid
    /// text keeps everything as it was and records why.
    pub fn from_text(self) -> Self {
        if self.text == pipeline_text(&self.stages) {
            return Self {
                mode: Mode::Stages,
                text_error: None,
                ..self
            };
        }
        match parse_pipeline_text(&self.text) {
            Ok(stages) => {
                let selected = Self::clamp(usize::MAX, stages.len());
                let (keys, next_key) = self.keyed(Vec::new(), stages.len());
                Self {
                    stages,
                    keys,
                    next_key,
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

    /// Whether a stage is turned off (text mode cannot keep that).
    pub fn has_disabled(&self) -> bool {
        self.stages.iter().any(|s| !s.enabled)
    }

    /// The pipeline now runs against another collection (or a new connection): an answer still
    /// in flight is stale and a shown preview no longer applies, so the pane is idle again. The
    /// draft itself stays.
    pub fn on_collection_changed(self) -> Self {
        Self {
            preview: PreviewState::Idle,
            queued: None,
            ..self
        }
        .changed()
    }

    /// Whether the tab should ask for a preview on its own: the pane is idle with a stage to run
    /// (an empty or all-disabled pipeline stays idle) and request `sent` (the last one asked for automatically) is not the current one, so an
    /// ask that could not run (no connection) is not repeated every frame.
    pub fn wants_auto_refresh(&self, sent: Option<u64>) -> bool {
        self.preview == PreviewState::Idle
            && sent != Some(self.request)
            && !matches!(
                preview_pipeline(&self.stages, self.selected, PREVIEW_LIMIT),
                Err(PipelineError::Empty)
            )
    }

    /// Starts a preview of the selected stage: the numbered request and the read-only pipeline
    /// to send, or `None` when nothing is sent: a pipeline with no runnable stage resets the pane to
    /// idle (dropping the old documents), any other refusal is shown as `Failed`.
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
            // Nothing to run: no error and no stale documents, just the idle pane.
            Err(PipelineError::Empty) => {
                let next = Self {
                    preview: PreviewState::Idle,
                    queued: None,
                    request,
                    ..self
                };
                (next, None)
            }
            Err(error) => {
                let preview = PreviewState::Failed {
                    error: PreviewError::Pipeline(error),
                    last,
                };
                let next = Self {
                    preview,
                    queued: None,
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
