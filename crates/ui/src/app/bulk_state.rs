//! The open bulk flow together with what the reducer must remember to trust its answers.
use crate::viewmodel::BulkFlow;
use uuid::Uuid;

/// A bulk flow and the bookkeeping around it. Not part of a session view: the flow names its
/// session, and every handler runs inside that session.
#[derive(Clone, Debug, PartialEq)]
pub struct BulkState {
    pub flow: BulkFlow,
    /// The session the flow was opened on.
    pub session: Uuid,
    /// The connect epoch of that session when the flow opened: a reconnect bumps it, so a flow
    /// never outlives the opening it was aimed at.
    pub epoch: u64,
    /// The id of the count request still awaited. An answer with another id is stale.
    pub count_request: Option<u64>,
    /// The id of the preview (or sample) request still awaited; dropped by Back so the answer of
    /// a preview of an older definition can never be applied.
    pub preview_request: Option<u64>,
    /// The id of the undo preview still awaited.
    pub undo_request: Option<u64>,
}

impl BulkState {
    /// The same state with `flow` in place of its own.
    pub fn with_flow(self, flow: BulkFlow) -> Self {
        Self { flow, ..self }
    }
}
