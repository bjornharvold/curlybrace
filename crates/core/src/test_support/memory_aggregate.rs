//! Scripted aggregate preview of the in-memory store: one canned answer, and a record of the
//! pipelines that reached it (a refused pipeline is never recorded).

use bson::Document;

use super::memory_store::{InMemoryStore, poisoned};
use crate::error::{CoreError, Result};
use crate::explain::ExplainVerbosity;
use crate::mongo_pipeline::ensure_read_only;
use crate::pipeline::{AppliedOptions, Operation, ensure_run_shape};
use crate::store::{Explain, Namespace};

/// What is scripted and what was received.
#[derive(Default)]
pub(super) struct AggregateScript {
    answer: Vec<Document>,
    received: Vec<Vec<Document>>,
    /// Pipelines that reached `aggregate_write` with a valid run shape.
    written: Vec<Vec<Document>>,
    write_error: Option<CoreError>,
    /// The explain document `aggregate_explain` answers; `None` is unsupported.
    explain_answer: Option<Document>,
    /// Pipelines that reached `aggregate_explain` (not refused), with their verbosity.
    explained: Vec<(Vec<Document>, ExplainVerbosity)>,
    /// The options each accepted call received, in call order.
    options: Vec<(Operation, AppliedOptions)>,
}

impl InMemoryStore {
    /// Scripts the documents `aggregate_preview` answers, whatever the pipeline.
    pub fn with_aggregate(self, results: Vec<Document>) -> Self {
        self.aggregate
            .lock()
            .expect("fresh mutex cannot be poisoned")
            .answer = results;
        self
    }

    /// The pipelines that reached `aggregate_preview`, in order. Refused ones are absent.
    pub fn received_pipelines(&self) -> Vec<Vec<Document>> {
        self.aggregate
            .lock()
            .map(|script| script.received.clone())
            .unwrap_or_default()
    }

    /// Makes `aggregate_write` fail with `error` after recording the pipeline.
    pub fn with_aggregate_write_error(self, error: CoreError) -> Self {
        self.aggregate
            .lock()
            .expect("fresh mutex cannot be poisoned")
            .write_error = Some(error);
        self
    }

    /// The pipelines that reached `aggregate_write`, in order. Refused ones are absent.
    pub fn written_pipelines(&self) -> Vec<Vec<Document>> {
        self.aggregate
            .lock()
            .map(|script| script.written.clone())
            .unwrap_or_default()
    }

    /// Scripts the document `aggregate_explain` answers, whatever the pipeline.
    pub fn with_aggregate_explain(self, document: Document) -> Self {
        self.aggregate
            .lock()
            .expect("fresh mutex cannot be poisoned")
            .explain_answer = Some(document);
        self
    }

    /// The pipelines that reached `aggregate_explain` with their verbosity, in order. Refused
    /// ones are absent.
    pub fn explained_pipelines(&self) -> Vec<(Vec<Document>, ExplainVerbosity)> {
        self.aggregate
            .lock()
            .map(|script| script.explained.clone())
            .unwrap_or_default()
    }

    /// The options each preview, explain and write call received, in order, as the operation
    /// that sends them: a count is a preview to the store, so it is recorded as `Preview`.
    /// Refused calls are absent.
    pub fn received_options(&self) -> Vec<(Operation, AppliedOptions)> {
        self.aggregate
            .lock()
            .map(|script| script.options.clone())
            .unwrap_or_default()
    }

    pub(super) fn run_aggregate_explain(
        &self,
        pipeline: Vec<Document>,
        verbosity: ExplainVerbosity,
        options: &AppliedOptions,
    ) -> Result<Explain> {
        ensure_read_only(&pipeline)?;
        let mut script = self.aggregate.lock().map_err(|_| poisoned())?;
        script.explained.push((pipeline, verbosity));
        script.options.push((Operation::Explain, options.clone()));
        script
            .explain_answer
            .clone()
            .map(|document| Explain {
                document,
                plan_only: verbosity == ExplainVerbosity::QueryPlanner,
            })
            .ok_or_else(|| CoreError::Unsupported("this store cannot explain a pipeline".into()))
    }

    pub(super) fn run_aggregate_write(
        &self,
        ns: &Namespace,
        pipeline: Vec<Document>,
        options: &AppliedOptions,
    ) -> Result<()> {
        ensure_run_shape(ns, &pipeline)?;
        let mut script = self.aggregate.lock().map_err(|_| poisoned())?;
        script.written.push(pipeline);
        script.options.push((Operation::Run, options.clone()));
        script.write_error.clone().map_or(Ok(()), Err)
    }

    pub(super) fn run_aggregate(
        &self,
        pipeline: Vec<Document>,
        options: &AppliedOptions,
    ) -> Result<Vec<Document>> {
        ensure_read_only(&pipeline)?;
        let mut script = self.aggregate.lock().map_err(|_| poisoned())?;
        script.received.push(pipeline);
        script.options.push((Operation::Preview, options.clone()));
        Ok(script.answer.clone())
    }
}
