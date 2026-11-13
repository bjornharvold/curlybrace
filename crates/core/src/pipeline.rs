//! Aggregation pipeline model: parse stage text and build a read-only preview.
//!
//! Stage text is canonical extended JSON, converted like `query::parse_doc`
//! (`serde_json` then `Bson::try_from`); serde's recursion limit (128) bounds
//! nesting, so deeply nested input is an error, never a stack overflow.
//! Pipeline text is user data and is never logged here.

use bson::{Bson, Document};
use serde_json::Value;

/// Documents shown by a preview.
pub const PREVIEW_LIMIT: i64 = 20;
/// Server-side time bound for a preview, in milliseconds.
pub const PREVIEW_MAX_TIME_MS: u64 = 10_000;
/// Stages that write; a preview never runs them.
pub const WRITE_STAGES: [&str; 2] = ["$out", "$merge"];

/// One pipeline stage as the author typed it.
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub text: String,
    pub enabled: bool,
}

/// Why pipeline text cannot be previewed. The UI maps each variant to an i18n key;
/// there is deliberately no English `Display`.
#[derive(Clone, Debug, PartialEq)]
pub enum PipelineError {
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },
    NotAnArray,
    /// Not an object with exactly one `$`-prefixed key.
    NotAStage {
        index: usize,
    },
    WriteStageNotLast {
        index: usize,
        name: String,
    },
    NestedWriteStage {
        index: usize,
        name: String,
    },
    Empty,
}

fn syntax(e: &serde_json::Error) -> PipelineError {
    PipelineError::Syntax {
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    }
}

/// Parses a JSON array of stage objects into enabled stages (2-space pretty JSON each).
pub fn parse_pipeline_text(text: &str) -> Result<Vec<Stage>, PipelineError> {
    let value: Value = serde_json::from_str(text).map_err(|e| syntax(&e))?;
    let Value::Array(items) = value else {
        return Err(PipelineError::NotAnArray);
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            if !item.is_object() {
                return Err(PipelineError::NotAStage { index });
            }
            let text = serde_json::to_string_pretty(item)
                .map_err(|_| PipelineError::NotAStage { index })?;
            Ok(Stage {
                text,
                enabled: true,
            })
        })
        .collect()
}

/// The pipeline as a JSON array text holding ALL stages, disabled ones too.
///
/// A text round trip loses the disabled flag (every stage comes back enabled).
/// Stage texts are embedded verbatim, so a stage that does not parse stays
/// visible and its error is reported when the text is parsed back.
pub fn pipeline_text(stages: &[Stage]) -> String {
    if stages.is_empty() {
        return "[]".to_string();
    }
    let body = stages
        .iter()
        .map(|s| {
            s.text
                .trim()
                .lines()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("[\n{body}\n]")
}

/// The one `$`-prefixed key of a stage text, when it parses as such.
pub fn stage_name(text: &str) -> Option<String> {
    parse_stage(text).map(|(name, _)| name)
}

fn parse_stage(text: &str) -> Option<(String, Document)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let Bson::Document(doc) = Bson::try_from(value).ok()? else {
        return None;
    };
    let mut keys = doc.keys();
    let name = keys.next()?.clone();
    if keys.next().is_some() || !name.starts_with('$') {
        return None;
    }
    Some((name, doc))
}

/// What to send for a preview, and which stages were not run.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewPlan {
    pub pipeline: Vec<Document>,
    pub not_run: Vec<String>,
}

fn is_write(name: &str) -> bool {
    WRITE_STAGES.contains(&name)
}

/// A write stage inside a list of stages (a sub-pipeline array).
fn find_write_in_stages(b: &Bson) -> Option<String> {
    match b {
        Bson::Array(items) => items.iter().find_map(|item| match item {
            Bson::Document(d) => find_write_in_stage(d),
            _ => None,
        }),
        _ => None,
    }
}

/// A write stage at the position of `stage` itself or in a sub-pipeline of it.
///
/// Sub-pipelines live only in `$facet` (each value), `$lookup.pipeline` and
/// `$unionWith.pipeline`. A `$out`/`$merge` key anywhere else is data.
fn find_write_in_stage(stage: &Document) -> Option<String> {
    stage.iter().find_map(|(key, value)| {
        if is_write(key) {
            return Some(key.clone());
        }
        match (key.as_str(), value) {
            ("$facet", Bson::Document(facets)) => facets.values().find_map(find_write_in_stages),
            ("$lookup" | "$unionWith", Bson::Document(inner)) => {
                inner.get("pipeline").and_then(find_write_in_stages)
            }
            _ => None,
        }
    })
}

/// A write stage (`$out`/`$merge`) at a stage position of the pipeline or of a
/// sub-pipeline (`$facet`, `$lookup`, `$unionWith`), by name.
pub fn contains_write_stage(pipeline: &[Document]) -> Option<String> {
    pipeline.iter().find_map(find_write_in_stage)
}

/// Builds the read-only pipeline that previews stages `0..=upto`.
///
/// Only enabled stages count. A trailing write stage is dropped and named in
/// `not_run`; a write stage anywhere else, or inside a sub-pipeline of a stage, is refused.
/// `{ "$limit": limit }` is appended.
pub fn preview_pipeline(
    stages: &[Stage],
    upto: usize,
    limit: i64,
) -> Result<PreviewPlan, PipelineError> {
    let mut parsed: Vec<(usize, String, Document)> = Vec::new();
    for (index, stage) in stages.iter().enumerate().take(upto.saturating_add(1)) {
        if !stage.enabled {
            continue;
        }
        let (name, doc) = parse_stage(&stage.text).ok_or(PipelineError::NotAStage { index })?;
        parsed.push((index, name, doc));
    }
    let mut not_run = Vec::new();
    if parsed.last().is_some_and(|(_, name, _)| is_write(name)) {
        if let Some((_, name, _)) = parsed.pop() {
            not_run.push(name);
        }
    }
    for (index, name, doc) in &parsed {
        if is_write(name) {
            return Err(PipelineError::WriteStageNotLast {
                index: *index,
                name: name.clone(),
            });
        }
        if let Some(nested) = find_write_in_stage(doc) {
            return Err(PipelineError::NestedWriteStage {
                index: *index,
                name: nested,
            });
        }
    }
    if parsed.is_empty() {
        return Err(PipelineError::Empty);
    }
    let mut pipeline: Vec<Document> = parsed.into_iter().map(|(_, _, d)| d).collect();
    pipeline.push(bson::doc! { "$limit": limit });
    Ok(PreviewPlan { pipeline, not_run })
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
