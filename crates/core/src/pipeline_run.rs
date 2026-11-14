//! The confirmed write run of an aggregation: a pipeline ending in `$out` or `$merge`.
//!
//! The preview (`pipeline::preview_pipeline`) refuses every write stage; this module is the one
//! place that accepts exactly one, as the last enabled stage. Pipeline text is user data: no
//! message here names anything from it but the stage kind.

use bson::{Bson, Document};

use crate::admin::RESERVED_DATABASES;
use crate::error::CoreError;
use crate::pipeline::{PipelineError, Stage, find_write_in_stage, is_write, parse_stage};
use crate::store::Namespace;

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

/// Why a list of stages is not the shape of a run. Positions index the list checked.
enum Misshape {
    Empty,
    /// A write stage before the last position.
    WriteNotLast {
        at: usize,
        name: String,
    },
    /// A write stage inside a sub-pipeline of an earlier stage.
    NestedWrite {
        at: usize,
        name: String,
    },
    /// The last stage is not a write stage.
    NoWrite,
}

/// The one check of a run's shape, shared by the plan (`run_pipeline`) and the store layer
/// (`ensure_run_shape`) so they cannot drift: stages as `(name, body)`, the last one the only
/// write stage and none nested in an earlier one. The target is not looked at here.
fn check_shape(stages: &[(&str, &Document)]) -> Result<(), Misshape> {
    let Some(((last_name, _), rest)) = stages.split_last() else {
        return Err(Misshape::Empty);
    };
    for (at, (name, doc)) in rest.iter().enumerate() {
        if let Some(found) = find_write_in_stage(doc) {
            return Err(match is_write(name) {
                true => Misshape::WriteNotLast { at, name: found },
                false => Misshape::NestedWrite { at, name: found },
            });
        }
    }
    match is_write(last_name) {
        true => Ok(()),
        false => Err(Misshape::NoWrite),
    }
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
    let shape: Vec<(&str, &Document)> = parsed.iter().map(|(_, n, d)| (n.as_str(), d)).collect();
    check_shape(&shape).map_err(|misshape| match misshape {
        Misshape::Empty => PipelineError::Empty,
        Misshape::WriteNotLast { at, name } => PipelineError::WriteStageNotLast {
            index: parsed[at].0,
            name,
        },
        Misshape::NestedWrite { at, name } => PipelineError::NestedWriteStage {
            index: parsed[at].0,
            name,
        },
        Misshape::NoWrite => PipelineError::NoWriteStage,
    })?;
    let last_index = parsed.last().map_or(0, |(index, _, _)| *index);
    let pipeline: Vec<Document> = parsed.into_iter().map(|(_, _, d)| d).collect();
    let (stage, target) =
        run_target(&pipeline).ok_or(PipelineError::TargetUnresolved { index: last_index })?;
    if is_reserved_target(&target) {
        return Err(PipelineError::ReservedTargetDatabase { index: last_index });
    }
    Ok(RunPlan {
        pipeline,
        stage,
        target,
    })
}

/// Whether the target names a database the server owns (`admin`, `local`, `config`). A target
/// without a database is the source's, which the store layer and the runtime check where they
/// know the source.
pub(crate) fn is_reserved_target(target: &RunTarget) -> bool {
    target.db.as_deref().is_some_and(is_reserved_db)
}

fn is_reserved_db(db: &str) -> bool {
    RESERVED_DATABASES.contains(&db)
}

fn refusal(reason: &str) -> CoreError {
    CoreError::InvalidDocument(format!("a run needs {reason}"))
}

/// Defence in depth for the store layer: exactly one write stage, at stage position, last, none
/// nested, with a resolvable target, in a database the server does not own: the effective one
/// is the target's own, else the source's (`ns.db`). The message names the stage, never the
/// pipeline text.
pub fn ensure_run_shape(ns: &Namespace, pipeline: &[Document]) -> Result<(), CoreError> {
    let named: Vec<(&str, &Document)> = pipeline
        .iter()
        .map(|doc| (doc.keys().next().map_or("", String::as_str), doc))
        .collect();
    let target = run_target(pipeline);
    let shape = check_shape(&named);
    match (shape, target) {
        (Err(Misshape::Empty), _) => Err(refusal("at least one stage")),
        (Err(Misshape::WriteNotLast { name, .. } | Misshape::NestedWrite { name, .. }), _) => {
            Err(refusal(&format!(
                "the write stage {name} to be the one and only, as the last stage"
            )))
        }
        (_, Some((_, target))) => match is_reserved_db(target.db.as_deref().unwrap_or(&ns.db)) {
            true => Err(CoreError::ReservedDatabase),
            false => Ok(()),
        },
        (_, None) => Err(refusal(
            match pipeline.last().and_then(find_write_in_stage) {
                Some(_) => "a single write stage with a usable target collection",
                None => "$out or $merge as its last stage",
            },
        )),
    }
}

#[cfg(test)]
#[path = "pipeline_run_tests.rs"]
mod tests;
