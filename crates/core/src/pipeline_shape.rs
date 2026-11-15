//! The one check of where write stages (`$out` / `$merge`) may be in a list of stages, shared by
//! the preview (`pipeline::preview_pipeline`), the code export (`pipeline::export_stages`) and
//! the write run (`pipeline_run`) so the three cannot drift. Each keeps its own policy for the
//! ending and its own error mapping; the placement rules are here once.

use bson::Document;

use crate::pipeline::{PipelineError, find_write_in_stage, is_write};

/// What the last stage of the list must be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ending {
    /// A write stage (a run).
    Write,
    /// Anything; a write stage there is allowed (kept by an export, dropped by a preview).
    Any,
}

/// Why a list of stages has the wrong shape. Positions index the list checked.
#[derive(Debug)]
pub(crate) enum Misshape {
    Empty,
    /// A write stage before the last position.
    WriteNotLast {
        at: usize,
        name: String,
    },
    /// A write stage inside a sub-pipeline of a stage that is not the trailing write stage.
    NestedWrite {
        at: usize,
        name: String,
    },
    /// The last stage is not a write stage, and the ending demands one.
    NoWrite,
}

/// Checks `stages` as `(name, body)`: no write stage before the last, none nested in a stage
/// (the trailing write stage itself is not looked into), and the ending as asked. Errors are
/// reported in stage order, and a missing write stage only after every placement was fine.
pub(crate) fn check_shape(stages: &[(&str, &Document)], ending: Ending) -> Result<(), Misshape> {
    let Some(((last_name, _), rest)) = stages.split_last() else {
        return Err(Misshape::Empty);
    };
    let ends_in_write = is_write(last_name);
    let checked = match (ending, ends_in_write) {
        (Ending::Any, false) => stages,
        _ => rest,
    };
    for (at, (name, doc)) in checked.iter().enumerate() {
        if let Some(found) = find_write_in_stage(doc) {
            return Err(match is_write(name) {
                true => Misshape::WriteNotLast { at, name: found },
                false => Misshape::NestedWrite { at, name: found },
            });
        }
    }
    match (ending, ends_in_write) {
        (Ending::Write, false) => Err(Misshape::NoWrite),
        _ => Ok(()),
    }
}

/// [`check_shape`] over stages that were parsed with their position in the whole list, mapped to
/// the [`PipelineError`] that names that position.
pub(crate) fn check_parsed(
    parsed: &[(usize, String, Document)],
    ending: Ending,
) -> Result<(), PipelineError> {
    let shape: Vec<(&str, &Document)> = parsed.iter().map(|(_, n, d)| (n.as_str(), d)).collect();
    check_shape(&shape, ending).map_err(|misshape| match misshape {
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
    })
}
