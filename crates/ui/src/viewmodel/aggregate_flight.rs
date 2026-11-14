//! The one preview a session keeps in flight. A preview can run on the server for up to its
//! time limit, so while one is out a newer request is only remembered (the newest wins, the
//! ones before it are never sent) and goes out when the answer of the one in flight arrives,
//! fresh or stale, a success or a failure. Every op returns a new state.
use super::aggregate::AggregateState;
use bson::Document;
use curlybrace_core::store::Namespace;

/// A planned preview waiting for the one in flight to answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Queued {
    pub request: u64,
    pub ns: Namespace,
    pub pipeline: Vec<Document>,
}

/// A preview to send: its request number and read-only pipeline.
pub type Outgoing = (u64, Vec<Document>);

impl AggregateState {
    /// Offers the planned preview `request`: sent now when nothing is in flight, else it
    /// replaces whatever waited before it.
    pub fn offered(
        self,
        request: u64,
        ns: Namespace,
        pipeline: Vec<Document>,
    ) -> (Self, Option<Outgoing>) {
        match self.in_flight {
            None => (
                Self {
                    in_flight: Some(request),
                    queued: None,
                    ..self
                },
                Some((request, pipeline)),
            ),
            Some(_) => (
                Self {
                    queued: Some(Queued {
                        request,
                        ns,
                        pipeline,
                    }),
                    ..self
                },
                None,
            ),
        }
    }

    /// The answer to `request` arrived (any outcome). When it is the one in flight the slot is
    /// free again and the waiting preview, if it is still the current request for `ns`, goes out.
    pub fn released(self, request: u64, ns: Option<&Namespace>) -> (Self, Option<Outgoing>) {
        match self.in_flight == Some(request) {
            true => Self {
                in_flight: None,
                ..self
            }
            .flushed(ns),
            false => (self, None),
        }
    }

    /// Sends the waiting preview when the slot is free and it still matches the current request
    /// and collection `ns`; a waiting preview that no longer does is dropped.
    pub fn flushed(self, ns: Option<&Namespace>) -> (Self, Option<Outgoing>) {
        if self.in_flight.is_some() {
            return (self, None);
        }
        let current = self.request;
        match self.queued {
            Some(q) if q.request == current && Some(&q.ns) == ns => (
                Self {
                    in_flight: Some(q.request),
                    queued: None,
                    ..self
                },
                Some((q.request, q.pipeline)),
            ),
            _ => (
                Self {
                    queued: None,
                    ..self
                },
                None,
            ),
        }
    }

    /// The connection is gone or replaced: nothing is in flight on it any more, and the waiting
    /// preview belonged to it. The preview is stale (see `on_collection_changed`).
    pub fn on_connection_reset(self) -> Self {
        Self {
            in_flight: None,
            queued: None,
            run: self.run.connection_reset(),
            ..self
        }
        .on_collection_changed()
    }

    /// The previews sent so far for `request` were not wanted after all (a frame of several
    /// changes sends only the last): its slot is free again.
    pub fn unsent(self, request: u64) -> Self {
        match self.in_flight == Some(request) {
            true => Self {
                in_flight: None,
                ..self
            },
            false => self,
        }
    }
}
