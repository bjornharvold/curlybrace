//! Applies chosen operations to the destination of compare pairs.
//!
//! Like `runner::start`, `dispatch` registers the job synchronously (session open and connected,
//! operations valid, every pair has results, slot free) so command ORDER decides outcomes; the
//! destination checks and the writes run in a task. Pairs apply one after another in the order
//! they first appear in the request. Every pair gets exactly one `ApplyPairDone` and the apply
//! exactly one `ApplyDone`, sent after the last `ApplyPairDone`, unless the session was closed
//! (then nothing more is sent: every emission goes through [`Session::emit_if_open`]). A refusal
//! found by the destination checks (before any write) is a `Failed` and has no `ApplyDone`.
//!
//! Keys and documents are data and are never logged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::compare::apply::{ApplyKind, ApplyOp, ApplyReport, Direction, IdPolicy, OpOutcome};
use crate::compare::apply_engine::{ApplyInput, apply_pair};
use crate::compare::engine::CancelFlag;
use crate::compare::key::compare_tuples;
use crate::compare::protocol::{CompareEvent, PairId, PairSpec, RunId, SessionId, Side};
use crate::compare::results::ResultSet;
use crate::error::{CoreError, Result};
use crate::store::{CollectionKind, DocumentStore, Namespace};

use super::runner::RunSlot;
use super::session::{ActiveRun, Session};
use super::{Events, Repaint, Shared, answer_closed, bounded, connections};

/// The operations of one pair, in request order.
type Group = (PairId, Vec<ApplyOp>);

/// Everything one apply needs to report back.
#[derive(Clone)]
struct ApplyContext {
    session: Arc<Session>,
    events: Events,
    repaint: Repaint,
    session_id: SessionId,
    run: RunId,
    /// Bound for the destination listing calls.
    limit: Duration,
}

impl ApplyContext {
    /// Sends `event` unless the session is closed.
    fn emit(&self, event: CompareEvent) -> bool {
        self.session
            .emit_if_open(&self.events, &self.repaint, event)
    }
}

fn internal_error() -> CoreError {
    CoreError::Database("internal error".into())
}

pub(super) fn dispatch(
    shared: &Shared,
    events: Events,
    repaint: Repaint,
    id: SessionId,
    run: RunId,
    ops: Vec<ApplyOp>,
    policy: IdPolicy,
) {
    let Some(session) = shared.compare.get(id).filter(|s| s.is_open()) else {
        return answer_closed(&events, &repaint, id, "compare_apply");
    };
    let ctx = ApplyContext {
        session,
        events,
        repaint,
        session_id: id,
        run,
        limit: shared.config.command_timeout,
    };
    if let Err(error) = start_apply(ctx.clone(), ops, policy) {
        ctx.emit(CompareEvent::Failed {
            session: id,
            context: "compare_apply",
            error,
        });
    }
}

/// The synchronous part: validates, takes the job slot, spawns the task.
fn start_apply(ctx: ApplyContext, ops: Vec<ApplyOp>, policy: IdPolicy) -> Result<()> {
    connections(&ctx.session)?;
    if ops.is_empty() {
        return Err(CoreError::Unsupported("nothing to apply".into()));
    }
    refuse_duplicates(&ops)?;
    let groups = group_by_pair(ops);
    require_results(&ctx.session, &groups)?;
    let cancel = CancelFlag::default();
    let slot = take_slot(&ctx.session, ctx.run, &cancel)?;
    tokio::spawn(run_apply_contained(ctx, groups, policy, cancel, slot));
    Ok(())
}

/// No two operations may act on the same `(pair, key)`; key equality is `compare_tuples`.
fn refuse_duplicates(ops: &[ApplyOp]) -> Result<()> {
    let mut order: Vec<&ApplyOp> = ops.iter().collect();
    order.sort_by(|a, b| a.pair.cmp(&b.pair).then(compare_tuples(&a.key, &b.key)));
    let repeated = order
        .windows(2)
        .any(|w| w[0].pair == w[1].pair && compare_tuples(&w[0].key, &w[1].key).is_eq());
    if repeated {
        return Err(CoreError::Unsupported("duplicate operation".into()));
    }
    Ok(())
}

/// Groups by pair in first-appearance order, keeping each pair's request order.
fn group_by_pair(ops: Vec<ApplyOp>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for op in ops {
        match groups.iter_mut().find(|(pair, _)| *pair == op.pair) {
            Some((_, list)) => list.push(op),
            None => groups.push((op.pair, vec![op])),
        }
    }
    groups
}

fn require_results(session: &Session, groups: &[Group]) -> Result<()> {
    let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
    if groups.iter().all(|(pair, _)| pairs.contains_key(pair)) {
        return Ok(());
    }
    Err(CoreError::Unsupported("no results for this pair".into()))
}

/// Takes the one job slot of the session (the same slot a compare run uses).
fn take_slot(session: &Arc<Session>, run: RunId, cancel: &CancelFlag) -> Result<RunSlot> {
    let mut slot = session.run.lock().unwrap_or_else(|e| e.into_inner());
    // Checked under the slot lock: a concurrent close either sees this job (and cancels it) or
    // has already marked the session closed.
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
    Ok(RunSlot::new(session.clone(), run))
}

/// Runs the apply in an inner task so a panic outside a pair still frees the slot (the guard
/// drops while unwinding) and ends the apply with its `ApplyDone` (unless the session closed).
async fn run_apply_contained(
    ctx: ApplyContext,
    groups: Vec<Group>,
    policy: IdPolicy,
    cancel: CancelFlag,
    slot: RunSlot,
) {
    let work = tokio::spawn(run_apply_all(ctx.clone(), groups, policy, cancel, slot));
    if work.await.is_err() {
        ctx.emit(CompareEvent::ApplyDone {
            session: ctx.session_id,
            run: ctx.run,
        });
    }
}

async fn run_apply_all(
    ctx: ApplyContext,
    groups: Vec<Group>,
    policy: IdPolicy,
    cancel: CancelFlag,
    slot: RunSlot,
) {
    if let Err(error) = check_destinations(&ctx, &groups).await {
        ctx.emit(CompareEvent::Failed {
            session: ctx.session_id,
            context: "compare_apply",
            error,
        });
        return;
    }
    for (pair, ops) in groups {
        let report = apply_group(&ctx, pair, ops, policy, &cancel).await;
        let done = CompareEvent::ApplyPairDone {
            session: ctx.session_id,
            run: ctx.run,
            pair,
            report,
        };
        if !ctx.emit(done) {
            return;
        }
    }
    slot.release();
    ctx.emit(CompareEvent::ApplyDone {
        session: ctx.session_id,
        run: ctx.run,
    });
}

/// One destination database to list, and what the operations want to do in its collections.
type Writes = HashMap<(Side, String), Vec<(String, ApplyKind)>>;

fn destination_side(op: &ApplyOp) -> Side {
    match op.direction {
        Direction::SourceToTarget => Side::Target,
        Direction::TargetToSource => Side::Source,
    }
}

/// What each operation will do to which destination collection, per `(side, database)`. A pair
/// discarded since the request is skipped: its operations fail as `NotFound` and write nothing.
fn planned_writes(session: &Session, groups: &[Group]) -> Writes {
    let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
    let mut writes = Writes::new();
    for (pair, ops) in groups {
        let Some(stored) = pairs.get(pair) else {
            continue;
        };
        for op in ops {
            let side = destination_side(op);
            let ns = destination_namespace(&stored.spec, side);
            writes
                .entry((side, ns.db.clone()))
                .or_default()
                .push((ns.collection.clone(), op.kind));
        }
    }
    writes
}

fn destination_namespace(spec: &PairSpec, side: Side) -> &Namespace {
    match side {
        Side::Source => &spec.source,
        Side::Target => &spec.target,
    }
}

/// Before any write: no destination may be a view, and a time-series destination takes inserts
/// only. A collection that does not exist yet is fine (the server creates it on insert).
async fn check_destinations(ctx: &ApplyContext, groups: &[Group]) -> Result<()> {
    let conns = connections(&ctx.session)?;
    for ((side, db), wanted) in planned_writes(&ctx.session, groups) {
        let store = conns.store(side);
        let kinds = bounded(ctx.limit, async { store.list_collection_kinds(&db).await }).await?;
        for (collection, kind) in wanted {
            let actual = kinds.iter().find(|(name, _)| *name == collection);
            let writable = match actual.map(|(_, k)| *k) {
                Some(CollectionKind::View) => false,
                Some(CollectionKind::Timeseries) => kind == ApplyKind::Insert,
                Some(CollectionKind::Collection) | None => true,
            };
            if !writable {
                return Err(CoreError::ApplyTargetNotWritable);
            }
        }
    }
    Ok(())
}

/// A report in which every operation of `ops` failed with `error`.
fn failed_report(ops: &[ApplyOp], error: &CoreError) -> ApplyReport {
    let mut report = ApplyReport::default();
    for op in ops {
        report.record(&op.key, &OpOutcome::Failed(error.clone()));
    }
    report
}

/// The pair's spec and results as they are now (`None` when it was discarded). The pairs lock
/// is released before this returns, so nothing is held across an await.
fn snapshot(session: &Session, pair: PairId) -> Option<(PairSpec, Arc<ResultSet>)> {
    let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
    pairs
        .get(&pair)
        .map(|stored| (stored.spec.clone(), stored.results.clone()))
}

async fn apply_group(
    ctx: &ApplyContext,
    pair: PairId,
    ops: Vec<ApplyOp>,
    policy: IdPolicy,
    cancel: &CancelFlag,
) -> ApplyReport {
    if cancel.is_cancelled() {
        return ApplyReport {
            not_run: ops.len() as u64,
            ..ApplyReport::default()
        };
    }
    let Some((spec, results)) = snapshot(&ctx.session, pair) else {
        return failed_report(&ops, &CoreError::NotFound);
    };
    let Ok(conns) = connections(&ctx.session) else {
        return failed_report(
            &ops,
            &CoreError::Unsupported("the comparison is closed".into()),
        );
    };
    let stores = (conns.source.clone(), conns.target.clone());
    let (task_ctx, task_cancel, task_ops) = (ctx.clone(), cancel.clone(), ops.clone());
    // The engine runs in its own task so a panic fails this pair only.
    let joined = tokio::spawn(async move {
        run_engine(
            &task_ctx,
            &spec,
            &results,
            stores,
            &task_ops,
            policy,
            &task_cancel,
        )
        .await
    })
    .await;
    match joined {
        Ok(Ok(report)) => report,
        Ok(Err(error)) => failed_report(&ops, &error),
        Err(_) => failed_report(&ops, &internal_error()),
    }
}

async fn run_engine(
    ctx: &ApplyContext,
    spec: &PairSpec,
    results: &ResultSet,
    stores: (Arc<dyn DocumentStore>, Arc<dyn DocumentStore>),
    ops: &[ApplyOp],
    policy: IdPolicy,
    cancel: &CancelFlag,
) -> Result<ApplyReport> {
    let total = ops.len() as u64;
    let input = ApplyInput {
        spec,
        results,
        source: stores.0,
        target: stores.1,
        ops,
        policy,
    };
    let mut progress = |done: u64| {
        ctx.emit(CompareEvent::ApplyProgress {
            session: ctx.session_id,
            run: ctx.run,
            pair: spec.id,
            done,
            total,
        });
    };
    apply_pair(input, cancel, &mut progress).await
}
