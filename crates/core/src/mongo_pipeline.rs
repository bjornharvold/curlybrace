//! The read-only aggregate preview of [`crate::mongo_store::MongoStore`]: the write-stage
//! check every store applies first, and the one aggregate call with its server-side limits.

use std::time::Duration;

use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::options::AggregateOptions;

use crate::driver_error::classify_driver_error;
use crate::error::{CoreError, Result};
use crate::explain::ExplainVerbosity;
use crate::pipeline::{contains_write_stage, ensure_run_shape};
use crate::store::{Explain, Namespace};

/// Refuses a pipeline holding `$out` or `$merge` at a stage position: the stage key itself, the
/// branches of `$facet`, `$lookup.pipeline` and `$unionWith.pipeline`. Defence in depth: the preview
/// plan already drops them, so this only fires if a caller skipped that step. The message names
/// the stage, never the pipeline text.
pub(crate) fn ensure_read_only(pipeline: &[Document]) -> Result<()> {
    match contains_write_stage(pipeline) {
        Some(stage) => Err(CoreError::InvalidDocument(format!(
            "a preview cannot run the write stage {stage}"
        ))),
        None => Ok(()),
    }
}

/// Options of a preview: no spill to disk, and a server-side limit so a heavy pipeline stops
/// instead of only being abandoned by the client.
fn preview_options(max_time: Duration) -> AggregateOptions {
    AggregateOptions::builder()
        .allow_disk_use(false)
        .max_time(max_time)
        .build()
}

/// Runs `pipeline` on `ns` and collects the result. The caller bounds the pipeline's size with
/// its own `$limit`.
pub(crate) async fn aggregate_preview(
    db: &Database,
    ns: &Namespace,
    pipeline: Vec<Document>,
    max_time: Duration,
) -> Result<Vec<Document>> {
    ensure_read_only(&pipeline)?;
    let cursor = db
        .collection::<Document>(&ns.collection)
        .aggregate(pipeline)
        .with_options(preview_options(max_time))
        .await
        .map_err(|e| classify_driver_error(&e))?;
    cursor
        .try_collect()
        .await
        .map_err(|e| classify_driver_error(&e))
}

/// The `explain` command for an aggregate: the pipeline with an empty cursor (an explain of an
/// aggregate must still say so), the verbosity, and the server-side time limit.
pub(crate) fn aggregate_explain_command(
    ns: &Namespace,
    pipeline: Vec<Document>,
    verbosity: ExplainVerbosity,
    max_time: Duration,
) -> Document {
    let max_time_ms = i64::try_from(max_time.as_millis()).unwrap_or(i64::MAX);
    doc! {
        "explain": {
            "aggregate": ns.collection.clone(),
            "pipeline": pipeline,
            "cursor": {},
        },
        "verbosity": verbosity.as_command_str(),
        "maxTimeMS": max_time_ms,
    }
}

/// Explains `pipeline` on `ns`. The write-stage check comes first, so a refused pipeline never
/// leaves the process. A server failure (a bad stage, a time limit) is classified like any other
/// store error; the explain document is returned as the server sent it.
pub(crate) async fn aggregate_explain(
    db: &Database,
    ns: &Namespace,
    pipeline: Vec<Document>,
    verbosity: ExplainVerbosity,
    max_time: Duration,
) -> Result<Explain> {
    ensure_read_only(&pipeline)?;
    let command = aggregate_explain_command(ns, pipeline, verbosity, max_time);
    let document = crate::mongo_explain::run_explain(db, command)
        .await
        .map_err(|e| classify_driver_error(&e))?;
    Ok(Explain {
        document,
        plan_only: verbosity == ExplainVerbosity::QueryPlanner,
    })
}

/// Options of a write run: only the server-side time limit (no options of the pipeline itself).
fn run_options(max_time: Duration) -> AggregateOptions {
    AggregateOptions::builder().max_time(max_time).build()
}

/// Runs a pipeline whose last stage is `$out`/`$merge` and drains its cursor, so the server
/// finishes the write before this returns. The shape is checked again here, before the server.
pub(crate) async fn aggregate_write(
    db: &Database,
    ns: &Namespace,
    pipeline: Vec<Document>,
    max_time: Duration,
) -> Result<()> {
    ensure_run_shape(ns, &pipeline)?;
    let mut cursor = db
        .collection::<Document>(&ns.collection)
        .aggregate(pipeline)
        .with_options(run_options(max_time))
        .await
        .map_err(|e| classify_driver_error(&e))?;
    while cursor
        .try_next()
        .await
        .map_err(|e| classify_driver_error(&e))?
        .is_some()
    {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn the_run_options_carry_only_the_time_limit() {
        let o = run_options(Duration::from_millis(900));
        assert_eq!(o.max_time, Some(Duration::from_millis(900)));
        assert_eq!(o.allow_disk_use, None);
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
            Duration::from_millis(10_000),
        );
        assert_eq!(
            planned,
            doc! {
                "explain": { "aggregate": "orders", "pipeline": pipeline.clone(), "cursor": {} },
                "verbosity": "queryPlanner",
                "maxTimeMS": 10_000_i64,
            }
        );
        let stats = aggregate_explain_command(
            &ns,
            pipeline,
            ExplainVerbosity::ExecutionStats,
            Duration::from_millis(1_500),
        );
        assert_eq!(stats.get_str("verbosity"), Ok("executionStats"));
        assert_eq!(stats.get_i64("maxTimeMS"), Ok(1_500));
        assert!(!stats.contains_key("allPlansExecution"));
    }

    #[test]
    fn the_options_forbid_disk_use_and_carry_the_time_limit() {
        let o = preview_options(Duration::from_millis(1500));
        assert_eq!(o.allow_disk_use, Some(false));
        assert_eq!(o.max_time, Some(Duration::from_millis(1500)));
    }
}
