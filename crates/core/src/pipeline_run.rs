//! The confirmed write run of an aggregation: a pipeline ending in `$out` or `$merge`.
//!
//! The preview (`pipeline::preview_pipeline`) refuses every write stage; this module is the one
//! place that accepts exactly one, as the last enabled stage. Pipeline text is user data: no
//! message here names anything from it but the stage kind.

use bson::{Bson, Document};

use crate::error::CoreError;
use crate::pipeline::{PipelineError, Stage, find_write_in_stage, is_write, parse_stage};

/// Server-side time bound for a write run, in milliseconds.
pub const AGGREGATE_RUN_MAX_TIME_MS: u64 = 60_000;

/// The collection a write stage writes to. `db: None` is the source database.
#[derive(Clone, Debug, PartialEq)]
pub struct RunTarget {
    pub db: Option<String>,
    pub collection: String,
}

/// What to send for a write run.
#[derive(Clone, Debug, PartialEq)]
pub struct RunPlan {
    /// Every enabled stage, the write stage last; no `$limit` is added.
    pub pipeline: Vec<Document>,
    /// `"$out"` or `"$merge"`.
    pub stage: String,
    pub target: RunTarget,
}

fn non_empty(b: &Bson) -> Option<String> {
    match b {
        Bson::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// `"coll"` or `{ db, coll }` (`db` optional, but a non-string or empty one is refused).
fn namespace_target(b: &Bson) -> Option<RunTarget> {
    match b {
        Bson::String(_) => Some(RunTarget {
            db: None,
            collection: non_empty(b)?,
        }),
        Bson::Document(d) => {
            let collection = non_empty(d.get("coll")?)?;
            let db = match d.get("db") {
                None => None,
                Some(v) => Some(non_empty(v)?),
            };
            Some(RunTarget { db, collection })
        }
        _ => None,
    }
}

fn stage_target(name: &str, body: &Bson) -> Option<RunTarget> {
    match (name, body) {
        ("$out", _) => namespace_target(body),
        ("$merge", Bson::Document(d)) => namespace_target(d.get("into")?),
        ("$merge", _) => namespace_target(body),
        _ => None,
    }
}

/// The write stage kind and target of the last stage of `pipeline`, when it is a well-formed
/// `$out`/`$merge` with a resolvable target.
pub fn run_target(pipeline: &[Document]) -> Option<(String, RunTarget)> {
    let last = pipeline.last()?;
    let mut entries = last.iter();
    let (name, body) = entries.next()?;
    if entries.next().is_some() || !is_write(name) {
        return None;
    }
    Some((name.clone(), stage_target(name, body)?))
}

/// Builds the pipeline of a write run from the enabled stages.
///
/// The last enabled stage must be `$out` or `$merge` with a resolvable target; any other write
/// stage (earlier, or inside `$facet`, `$lookup.pipeline`, `$unionWith.pipeline`) is refused.
pub fn run_pipeline(stages: &[Stage]) -> Result<RunPlan, PipelineError> {
    let mut parsed: Vec<(usize, String, Document)> = Vec::new();
    for (index, stage) in stages.iter().enumerate().filter(|(_, s)| s.enabled) {
        let (name, doc) = parse_stage(&stage.text).ok_or(PipelineError::NotAStage { index })?;
        parsed.push((index, name, doc));
    }
    let Some((last_index, last_name, _)) = parsed.last().cloned() else {
        return Err(PipelineError::Empty);
    };
    for (index, name, doc) in &parsed[..parsed.len() - 1] {
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
    if !is_write(&last_name) {
        return Err(PipelineError::NoWriteStage);
    }
    let pipeline: Vec<Document> = parsed.into_iter().map(|(_, _, d)| d).collect();
    let (stage, target) =
        run_target(&pipeline).ok_or(PipelineError::TargetUnresolved { index: last_index })?;
    Ok(RunPlan {
        pipeline,
        stage,
        target,
    })
}

fn refusal(reason: &str) -> CoreError {
    CoreError::InvalidDocument(format!("a run needs {reason}"))
}

/// Defence in depth for the store layer: exactly one write stage, at stage position, last, none
/// nested, with a resolvable target. The message names the stage, never the pipeline text.
pub fn ensure_run_shape(pipeline: &[Document]) -> Result<(), CoreError> {
    let Some((last, rest)) = pipeline.split_last() else {
        return Err(refusal("at least one stage"));
    };
    if let Some(name) = rest.iter().find_map(find_write_in_stage) {
        return Err(refusal(&format!(
            "the write stage {name} to be the one and only, as the last stage"
        )));
    }
    if run_target(pipeline).is_none() {
        let what = if find_write_in_stage(last).is_some() {
            "a single write stage with a usable target collection"
        } else {
            "$out or $merge as its last stage"
        };
        return Err(refusal(what));
    }
    Ok(())
}

#[cfg(test)]
#[path = "pipeline_run_tests.rs"]
mod tests;
