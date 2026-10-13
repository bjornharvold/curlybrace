//! Runs the pairs of one compare run and serves their stored results.
//!
//! Pairs run one after another; each pair's failure is its own (a panic in the engine is caught
//! and reported as that pair's failure). A cancel (or a close) ends the remaining pairs as
//! `Cancelled`. Every pair gets exactly one `PairDone` and the run exactly one `RunDone`, sent
//! after the last `PairDone`, unless the session was closed (then nothing more is sent: every
//! emission goes through [`Session::emit_if_open`]).

use std::collections::HashSet;
use std::sync::Arc;

use crate::compare::diff::diff_normalized;
use crate::compare::engine::{CancelFlag, EngineOutcome, PairInput, Progress, Summary, run_pair};
use crate::compare::key::{KeySpec, KeyTuple};
use crate::compare::normalize::normalize;
use crate::compare::protocol::{
    CompareEvent, PairId, PairOutcome, PairSpec, RowView, RunId, SessionId,
};
use crate::compare::results::Category;
use crate::compare::scan::fetch_one;
use crate::error::{CoreError, Result};
use crate::store::IndexStatus;

use super::session::{ActiveRun, Connections, Session, StoredPair};
use super::{Events, Repaint, Shared, connections};

/// The most rows one `Rows` answer carries, whatever the request asks for.
pub(super) const MAX_ROWS_PAGE: usize = 1000;

fn internal_error() -> CoreError {
    CoreError::Database("internal error".into())
}

fn no_results() -> CoreError {
    CoreError::Unsupported("no results for this pair".into())
}

pub(super) fn cancel(shared: &Shared, session: SessionId, run: RunId) {
    let Some(session) = shared.compare.get(session) else {
        return;
    };
    let guard = session.run.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(active) = guard.as_ref().filter(|a| a.id == run) {
        active.cancel.cancel();
    }
}

pub(super) fn discard(shared: &Shared, session: SessionId, pair: PairId) {
    if let Some(session) = shared.compare.get(session) {
        discard_pair(&session, pair);
    }
}

pub(super) fn discard_pair(session: &Session, pair: PairId) {
    // Dropping the stored set (after the lock is released) returns its share of the budget.
    let removed = session
        .pairs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&pair);
    drop(removed);
}

/// Frees the "a run is active" slot however the run task ends. It only ever clears the slot
/// holding its own run: after an explicit release a newer run may already own the slot, and the
/// later drop must not erase it.
pub(super) struct RunSlot {
    session: Arc<Session>,
    run: RunId,
}

impl RunSlot {
    pub(super) fn new(session: Arc<Session>, run: RunId) -> Self {
        Self { session, run }
    }

    pub(super) fn release(&self) {
        let mut slot = self.session.run.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|a| a.id == self.run) {
            *slot = None;
        }
    }
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        self.release();
    }
}

/// Everything one run needs to report back.
#[derive(Clone)]
struct RunContext {
    session: Arc<Session>,
    events: Events,
    repaint: Repaint,
    session_id: SessionId,
    run: RunId,
}

impl RunContext {
    /// Sends `event` unless the session is closed.
    fn emit(&self, event: CompareEvent) -> bool {
        self.session
            .emit_if_open(&self.events, &self.repaint, event)
    }
}

/// Checks what the user entered for a pair: the key (fields are public, so a spec can bypass
/// `KeySpec::new`) and the rule paths. `CompareSetupInvalid` when either is unusable.
pub(super) fn validate_spec(spec: &PairSpec) -> Result<()> {
    KeySpec::new(spec.key.fields.clone())?;
    spec.rules.validate()
}

pub(super) fn start(
    events: Events,
    repaint: Repaint,
    session: Arc<Session>,
    session_id: SessionId,
    run: RunId,
    pairs: Vec<PairSpec>,
) -> Result<()> {
    connections(&session)?;
    let mut seen = HashSet::new();
    if !pairs.iter().all(|p| seen.insert(p.id)) {
        return Err(CoreError::Unsupported("duplicate pair ids".into()));
    }
    pairs.iter().try_for_each(validate_spec)?;
    let cancel = CancelFlag::default();
    {
        let mut slot = session.run.lock().unwrap_or_else(|e| e.into_inner());
        // Checked under the slot lock: a concurrent close either sees this run (and cancels
        // it) or has already marked the session closed.
        if !session.is_open() {
            return Err(CoreError::Unsupported("the comparison is closed".into()));
        }
        if slot.is_some() {
            return Err(CoreError::Unsupported(
                "a comparison is already running".into(),
            ));
        }
        *slot = Some(ActiveRun {
            id: run,
            cancel: cancel.clone(),
        });
    }
    let ctx = RunContext {
        session,
        events,
        repaint,
        session_id,
        run,
    };
    tokio::spawn(run_contained(ctx, pairs, cancel));
    Ok(())
}

/// Runs the pairs in an inner task so a panic outside the engine still frees the run slot and
/// ends the run with its `RunDone` (unless the session was closed).
async fn run_contained(ctx: RunContext, pairs: Vec<PairSpec>, cancel: CancelFlag) {
    let work = tokio::spawn(run_all(ctx.clone(), pairs, cancel));
    if work.await.is_err() {
        // The slot guard inside `run_all` already freed the slot while unwinding.
        ctx.emit(CompareEvent::RunDone {
            session: ctx.session_id,
            run: ctx.run,
        });
    }
}

async fn run_all(ctx: RunContext, pairs: Vec<PairSpec>, cancel: CancelFlag) {
    let slot = RunSlot::new(ctx.session.clone(), ctx.run);
    for spec in pairs {
        let id = spec.id;
        let outcome = run_one(&ctx, spec, &cancel).await;
        let done = CompareEvent::PairDone {
            session: ctx.session_id,
            run: ctx.run,
            pair: id,
            outcome,
        };
        if !ctx.emit(done) {
            return;
        }
    }
    slot.release();
    ctx.emit(CompareEvent::RunDone {
        session: ctx.session_id,
        run: ctx.run,
    });
}

async fn run_one(ctx: &RunContext, spec: PairSpec, cancel: &CancelFlag) -> PairOutcome {
    // A re-run replaces the pair's results: drop the old set now (returning its budget share),
    // so a failed or cancelled re-run never leaves stale rows behind.
    discard_pair(&ctx.session, spec.id);
    if cancel.is_cancelled() {
        return PairOutcome::Cancelled(Summary::default());
    }
    let Ok(conns) = connections(&ctx.session) else {
        return PairOutcome::Failed(CoreError::Unsupported("the comparison is closed".into()));
    };
    let cap = ctx.session.budget.limits().max_unindexed_documents;
    match cancel
        .run_until_cancelled(index_guard(conns, &spec, cap))
        .await
    {
        None => return PairOutcome::Cancelled(Summary::default()),
        Some(Err(error)) => return PairOutcome::Failed(error),
        Some(Ok(())) => {}
    }
    let input = PairInput {
        source: conns.source.clone(),
        source_ns: spec.source.clone(),
        source_filter: spec.source_filter.clone(),
        target: conns.target.clone(),
        target_ns: spec.target.clone(),
        target_filter: spec.target_filter.clone(),
        key: spec.key.clone(),
        rules: spec.rules.clone(),
        budget: ctx.session.budget.clone(),
    };
    let (progress_ctx, pair_id, cancel_c) = (ctx.clone(), spec.id, cancel.clone());
    // The engine runs in its own task so a panic fails this pair only.
    let joined = tokio::spawn(async move {
        let mut report = |p: Progress| {
            progress_ctx.emit(CompareEvent::Progress {
                session: progress_ctx.session_id,
                run: progress_ctx.run,
                pair: pair_id,
                scanned_source: p.scanned_source,
                scanned_target: p.scanned_target,
                differences: p.differences,
            });
        };
        run_pair(input, &cancel_c, &mut report).await
    })
    .await;
    match joined {
        Ok(Ok((EngineOutcome::Completed, summary, results))) => {
            store_pair(&ctx.session, StoredPair { spec, results });
            PairOutcome::Completed(summary)
        }
        // A cancelled or failed pair keeps nothing; dropping its results returns the budget.
        Ok(Ok((EngineOutcome::Cancelled, summary, _discarded))) => PairOutcome::Cancelled(summary),
        Ok(Err(error)) => PairOutcome::Failed(error),
        Err(_) => PairOutcome::Failed(internal_error()),
    }
}

/// Refuses a pair that would sort a large collection without an index: re-checked at the start
/// of every pair (the preflight may be stale, or skipped). `CompareKeyNotIndexed` when a side has
/// no usable index and more than `cap` documents.
async fn index_guard(conns: &Connections, spec: &PairSpec, cap: u64) -> Result<()> {
    let sides = [(&conns.source, &spec.source), (&conns.target, &spec.target)];
    for (store, ns) in sides {
        if store.key_index_status(ns, &spec.key).await? == IndexStatus::NotIndexed
            && store.estimated_count(ns).await? > cap
        {
            return Err(CoreError::CompareKeyNotIndexed);
        }
    }
    Ok(())
}

/// Keeps a finished pair unless the session closed meanwhile (the check and the insert happen
/// under the pairs lock, which `close()` also takes after marking the session closed).
fn store_pair(session: &Session, stored: StoredPair) {
    let replaced = {
        let mut pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
        if session.is_open() {
            pairs.insert(stored.spec.id, stored)
        } else {
            Some(stored)
        }
    };
    // Dropped outside the lock; either way its budget share is returned.
    drop(replaced);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rows(
    session: &Session,
    events: &Events,
    repaint: &Repaint,
    session_id: SessionId,
    pair: PairId,
    category: Category,
    offset: usize,
    limit: usize,
) -> Result<()> {
    let limit = limit.min(MAX_ROWS_PAGE);
    let (total, rows) = {
        let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
        let stored = pairs.get(&pair).ok_or_else(no_results)?;
        let rows: Vec<RowView> = stored
            .results
            .page(category, offset, limit)
            .iter()
            .map(|e| RowView {
                key: e.key.clone(),
                has_source: e.source.is_some(),
                has_target: e.target.is_some(),
            })
            .collect();
        (stored.results.count(category), rows)
    };
    let answer = CompareEvent::Rows {
        session: session_id,
        pair,
        category,
        offset,
        total,
        rows,
    };
    session.emit_if_open(events, repaint, answer);
    Ok(())
}

/// Re-reads both documents for `key` and diffs them under the pair's rules. A document that no
/// longer matches the pair's filter is reported like a deleted one (absent on that side).
pub(super) async fn detail(
    session: &Session,
    session_id: SessionId,
    pair: PairId,
    key: KeyTuple,
) -> Result<CompareEvent> {
    let spec = {
        let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
        pairs.get(&pair).ok_or_else(no_results)?.spec.clone()
    };
    let conns = connections(session)?;
    let (source, target) = tokio::try_join!(
        fetch_one(
            &conns.source,
            &spec.source,
            &spec.source_filter,
            &spec.key,
            &key
        ),
        fetch_one(
            &conns.target,
            &spec.target,
            &spec.target_filter,
            &spec.key,
            &key
        )
    )?;
    let changes = match (&source, &target) {
        (Some(s), Some(t)) => diff_normalized(
            &normalize(s, &spec.rules)?,
            &normalize(t, &spec.rules)?,
            spec.rules.ignore_array_order,
        )?,
        _ => Vec::new(),
    };
    Ok(CompareEvent::Detail {
        session: session_id,
        pair,
        key,
        source,
        target,
        changes,
    })
}
