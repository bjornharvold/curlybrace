//! The aggregate operations of [`crate::mongo_store::MongoStore`]: the write-stage check every
//! store applies first, and the one aggregate call with its server-side limits and the options
//! (see `mongo_pipeline_options`).

use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Database;

use crate::error::{CoreError, Result};
use crate::explain::ExplainVerbosity;
use crate::mongo_explain::{explain_criteria, run_explain_on};
use crate::mongo_pipeline_options::{
    classify_with, explain_fields, read_criteria, read_options, write_options,
};
use crate::pipeline::{AppliedOptions, contains_write_stage, ensure_run_shape};
use crate::store::{Explain, Namespace};

/// Refuses a pipeline holding `$out` or `$merge` at a stage position: the stage key itself, the
/// branches of `$facet`, `$lookup.pipeline` and `$unionWith.pipeline`. Defence in depth: the preview
/// plan already drops them, so this only fires if a caller skipped that step. The message names
/// the stage, never the pipeline text.
pub(crate) fn ensure_read_only(pipeline: &[Document]) -> Result<()> {
    match contains_write_stage(pipeline) {
        Some(stage) => Err(CoreError::InvalidDocument(format!(
            "a read-only operation cannot run the write stage {stage}"
        ))),
        None => Ok(()),
    }
}

/// Runs `pipeline` on `ns` and collects the result. The caller bounds the pipeline's size with
/// its own `$limit`. The options are the ones a preview or a count sends (`read_options`: no
/// spill to disk, whatever `options` says).
pub(crate) async fn aggregate_preview(
    db: &Database,
    ns: &Namespace,
    pipeline: Vec<Document>,
    options: &AppliedOptions,
) -> Result<Vec<Document>> {
    ensure_read_only(&pipeline)?;
    let cursor = db
        .collection::<Document>(&ns.collection)
        .aggregate(pipeline)
        .with_options(read_options(options))
        .await
        .map_err(|e| classify_with(&e, options))?;
    cursor
        .try_collect()
        .await
        .map_err(|e| classify_with(&e, options))
}

/// The `explain` command for an aggregate: the pipeline with an empty cursor (an explain of an
/// aggregate must still say so) and the options inside it, the verbosity, and the server-side
/// time limit.
pub(crate) fn aggregate_explain_command(
    ns: &Namespace,
    pipeline: Vec<Document>,
    verbosity: ExplainVerbosity,
    options: &AppliedOptions,
) -> Document {
    let max_time_ms = i64::try_from(options.max_time.as_millis()).unwrap_or(i64::MAX);
    let mut inner = doc! {
        "aggregate": ns.collection.clone(),
        "pipeline": pipeline,
        "cursor": {},
    };
    inner.extend(explain_fields(options));
    doc! {
        "explain": inner,
        "verbosity": verbosity.as_command_str(),
        "maxTimeMS": max_time_ms,
    }
}

/// Explains `pipeline` on `ns`. The write-stage check comes first, so a refused pipeline never
/// leaves the process. A server failure (a bad stage, a time limit) is classified like any other
/// store error; the explain document is returned as the server sent it. It goes where the
/// options' read preference says, else where the connection's does.
pub(crate) async fn aggregate_explain(
    db: &Database,
    ns: &Namespace,
    pipeline: Vec<Document>,
    verbosity: ExplainVerbosity,
    options: &AppliedOptions,
) -> Result<Explain> {
    ensure_read_only(&pipeline)?;
    let command = aggregate_explain_command(ns, pipeline, verbosity, options);
    let criteria = read_criteria(options).or_else(|| explain_criteria(db));
    let document = run_explain_on(db, command, criteria)
        .await
        .map_err(|e| classify_with(&e, options))?;
    Ok(Explain {
        document,
        plan_only: verbosity == ExplainVerbosity::QueryPlanner,
    })
}

/// Runs a pipeline whose last stage is `$out`/`$merge` and drains its cursor, so the server
/// finishes the write before this returns. The shape is checked again here, before the server.
/// The run always selects the primary (`write_options`), whatever the connection's own read
/// preference or `options` say.
pub(crate) async fn aggregate_write(
    db: &Database,
    ns: &Namespace,
    pipeline: Vec<Document>,
    options: &AppliedOptions,
) -> Result<()> {
    ensure_run_shape(ns, &pipeline)?;
    let mut cursor = db
        .collection::<Document>(&ns.collection)
        .aggregate(pipeline)
        .with_options(write_options(options))
        .await
        .map_err(|e| classify_with(&e, options))?;
    while cursor
        .try_next()
        .await
        .map_err(|e| classify_with(&e, options))?
        .is_some()
    {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::Operation;
    use bson::doc;
    use std::time::Duration;

    fn bounded(op: Operation, ms: u64) -> AppliedOptions {
        AppliedOptions::bounded(op, Duration::from_millis(ms))
    }

    #[test]
    fn a_read_only_pipeline_passes() {
        assert!(ensure_read_only(&[doc! { "$match": { "a": 1 } }, doc! { "$limit": 20 }]).is_ok());
    }

    #[test]
    fn a_write_stage_at_any_depth_is_refused_without_the_pipeline_text() {
        let nested = doc! { "$facet": { "f": [ { "$out": "secret_target" } ] } };
        for pipeline in [vec![doc! { "$merge": "secret_target" }], vec![nested]] {
            match ensure_read_only(&pipeline) {
                Err(CoreError::InvalidDocument(m)) => {
                    assert!(m.contains("$out") || m.contains("$merge"));
                    assert!(!m.contains("secret_target"));
                    assert!(
                        !m.contains("preview"),
                        "the preview and the explain share it"
                    );
                }
                other => panic!("expected a refusal, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_explain_command_wraps_an_aggregate_with_verbosity_and_time_limit() {
        let ns = Namespace::new("shop", "orders");
        let pipeline = vec![
            doc! { "$match": { "a": 1 } },
            doc! { "$group": { "_id": "$b" } },
        ];
        let planned = aggregate_explain_command(
            &ns,
            pipeline.clone(),
            ExplainVerbosity::QueryPlanner,
            &bounded(Operation::Explain, 10_000),
        );
        assert_eq!(
            planned,
            doc! {
                "explain": {
                    "aggregate": "orders",
                    "pipeline": pipeline.clone(),
                    "cursor": {},
                },
                "verbosity": "queryPlanner",
                "maxTimeMS": 10_000_i64,
            }
        );
        let stats = aggregate_explain_command(
            &ns,
            pipeline.clone(),
            ExplainVerbosity::ExecutionStats,
            &bounded(Operation::Explain, 1_500),
        );
        assert_eq!(stats.get_str("verbosity"), Ok("executionStats"));
        assert_eq!(stats.get_i64("maxTimeMS"), Ok(1_500));
        assert!(!stats.contains_key("allPlansExecution"));
        // The server default decides disk use: forbidding it would fail a run that spills.
        assert!(!stats.contains_key("allowDiskUse"));
        assert!(
            !stats
                .get_document("explain")
                .is_ok_and(|e| e.contains_key("allowDiskUse"))
        );
        // The command adds no stage of its own to the pipeline (no write stage in particular),
        // and a pipeline that ends in a write stage is refused before a command is built.
        assert_eq!(
            stats
                .get_document("explain")
                .and_then(|e| e.get_array("pipeline"))
                .map(Vec::len),
            Ok(pipeline.len())
        );
        let mut with_write = pipeline;
        with_write.push(doc! { "$out": "target" });
        assert!(ensure_read_only(&with_write).is_err());
    }
}
