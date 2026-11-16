//! The pipeline a per-stage count sends: the enabled stages up to a stage, bounded by a `$limit`
//! and closed by `$count`. Stage text is user data and is never logged here.

use bson::{Bson, Document};

use crate::error::{CoreError, Result};
use crate::pipeline::{PipelineError, Stage, is_write, parse_stage, read_parsed};

/// Documents a count looks at before it gives up and reports "at least".
pub const COUNT_CAP: i64 = 100_000;

/// The field the closing `$count` stage writes.
const COUNT_FIELD: &str = "n";

/// Builds the read-only pipeline that counts the documents stage `upto` lets through: the
/// enabled stages `0..=upto`, then `{$limit: cap}` (so a huge collection stops at the cap) and
/// `{$count: "n"}`. A write stage at or before `upto` is refused, never dropped: a count of a
/// write stage has no meaning and nothing that writes is ever sent.
pub fn count_pipeline(
    stages: &[Stage],
    upto: usize,
    cap: i64,
) -> std::result::Result<Vec<Document>, PipelineError> {
    counted(stages, &parse_enabled(stages), upto, cap)
}

/// [`count_pipeline`] for every stage at once: entry `i` is exactly what
/// `count_pipeline(stages, i, cap)` returns, but each stage's text is parsed once instead of
/// once per later stage.
pub fn count_pipelines(
    stages: &[Stage],
    cap: i64,
) -> Vec<std::result::Result<Vec<Document>, PipelineError>> {
    let parsed = parse_enabled(stages);
    (0..stages.len())
        .map(|upto| counted(stages, &parsed, upto, cap))
        .collect()
}

/// Each stage parsed once. `None`: disabled. `Some(None)`: enabled and not a stage.
type Parsed = Vec<Option<Option<(String, Document)>>>;

fn parse_enabled(stages: &[Stage]) -> Parsed {
    stages
        .iter()
        .map(|s| s.enabled.then(|| parse_stage(&s.text)))
        .collect()
}

/// The count pipeline of stages `0..=upto` out of their parsed form. A write stage that is the
/// only enabled stage in range leaves nothing to read once dropped, which would read as `Empty`;
/// for a count it is still a write stage, so it is named as one.
fn counted(
    stages: &[Stage],
    parsed: &Parsed,
    upto: usize,
    cap: i64,
) -> std::result::Result<Vec<Document>, PipelineError> {
    let mut prefix = Vec::new();
    for (index, entry) in parsed.iter().enumerate().take(upto.saturating_add(1)) {
        match entry {
            None => {}
            Some(None) => return Err(PipelineError::NotAStage { index }),
            Some(Some((name, doc))) => prefix.push((index, name.clone(), doc.clone())),
        }
    }
    let lone_write = match prefix.as_slice() {
        [(index, name, _)] if is_write(name) => Some((*index, name.clone())),
        _ => None,
    };
    let (pipeline, not_run) = match (read_parsed(prefix), lone_write) {
        (Err(PipelineError::Empty), Some((index, name))) => {
            return Err(PipelineError::WriteStageNotLast { index, name });
        }
        (read, _) => read?,
    };
    closed(stages, upto, cap, pipeline, not_run)
}

/// The read stages closed by `$limit` and `$count`; a dropped trailing write stage is an error.
fn closed(
    stages: &[Stage],
    upto: usize,
    cap: i64,
    mut pipeline: Vec<Document>,
    not_run: Vec<String>,
) -> std::result::Result<Vec<Document>, PipelineError> {
    if let Some(name) = not_run.into_iter().next() {
        // `read_stages` drops a trailing write stage: that is the last enabled one in range.
        let index = stages
            .iter()
            .enumerate()
            .take(upto.saturating_add(1))
            .filter(|(_, s)| s.enabled)
            .map(|(i, _)| i)
            .next_back()
            .unwrap_or(upto);
        return Err(PipelineError::WriteStageNotLast { index, name });
    }
    pipeline.push(bson::doc! { "$limit": cap });
    pipeline.push(bson::doc! { "$count": COUNT_FIELD });
    Ok(pipeline)
}

/// Reads the count out of the answer of a [`count_pipeline`] pipeline. `$count` over nothing
/// returns no document, which is a count of 0. Anything that is not one document with a
/// non-negative whole number `n` is an error.
pub fn count_of(docs: &[Document]) -> Result<u64> {
    let Some(first) = docs.first() else {
        return Ok(0);
    };
    if docs.len() > 1 {
        return Err(unreadable());
    }
    match first.get(COUNT_FIELD) {
        Some(Bson::Int32(n)) => u64::try_from(*n).map_err(|_| unreadable()),
        Some(Bson::Int64(n)) => u64::try_from(*n).map_err(|_| unreadable()),
        Some(Bson::Double(n)) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => Ok(*n as u64),
        _ => Err(unreadable()),
    }
}

fn unreadable() -> CoreError {
    CoreError::InvalidDocument("the count answer has no readable n".into())
}

#[cfg(test)]
#[path = "pipeline_count_tests.rs"]
mod tests;
