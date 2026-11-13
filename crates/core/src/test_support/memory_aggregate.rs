//! Scripted aggregate preview of the in-memory store: one canned answer, and a record of the
//! pipelines that reached it (a refused pipeline is never recorded).

use std::time::Duration;

use bson::Document;

use super::memory_store::{InMemoryStore, poisoned};
use crate::error::Result;
use crate::mongo_pipeline::ensure_read_only;

/// What is scripted and what was received.
#[derive(Default)]
pub(super) struct AggregateScript {
    answer: Vec<Document>,
    received: Vec<Vec<Document>>,
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

    pub(super) fn run_aggregate(
        &self,
        pipeline: Vec<Document>,
        _max_time: Duration,
    ) -> Result<Vec<Document>> {
        ensure_read_only(&pipeline)?;
        let mut script = self.aggregate.lock().map_err(|_| poisoned())?;
        script.received.push(pipeline);
        Ok(script.answer.clone())
    }
}
