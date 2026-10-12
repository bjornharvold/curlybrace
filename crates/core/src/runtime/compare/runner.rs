//! Runs pairs and serves their results (completed in Task 8).

use std::sync::Arc;

use crate::compare::key::KeyTuple;
use crate::compare::protocol::{PairId, PairSpec, RunId, SessionId};
use crate::compare::results::Category;
use crate::error::{CoreError, Result};

use super::session::Session;
use super::{Events, Repaint, Shared};

pub(super) fn cancel(_: &Shared, _: SessionId, _: RunId) {}
pub(super) fn discard(_: &Shared, _: SessionId, _: PairId) {}

pub(super) fn start(
    _: Arc<Shared>,
    _: Events,
    _: Repaint,
    _: Arc<Session>,
    _: SessionId,
    _: RunId,
    _: Vec<PairSpec>,
) -> Result<()> {
    Err(CoreError::Unsupported("not implemented yet".into()))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rows(
    _: &Session,
    _: &Events,
    _: &Repaint,
    _: SessionId,
    _: PairId,
    _: Category,
    _: usize,
    _: usize,
) -> Result<()> {
    Err(CoreError::Unsupported("not implemented yet".into()))
}

pub(super) async fn detail(
    _: &Session,
    _: SessionId,
    _: PairId,
    _: KeyTuple,
) -> Result<crate::compare::protocol::CompareEvent> {
    Err(CoreError::Unsupported("not implemented yet".into()))
}
