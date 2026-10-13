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
//! A pair discarded before its turn fails its ops as `NotFound`; a pair already running finishes
//! on the results snapshot it started with (batches are not re-checked against a discard). A
//! panic in a pair fails that pair and cancels the rest (`not_run`); `ApplyDone` still follows.
//! `Failed` carries no run id, so the UI must send one `Apply` at a time.
//!
//! Bulk expansion and the duplicate check run synchronously on the command loop on purpose
//! (command order decides outcomes); they take time proportional to the number of rows.
//!
//! Keys and documents are data and are never logged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::compare::apply::{ApplyKind, ApplyOp, ApplyReport, IdPolicy, OpOutcome, kind_fits};
use crate::compare::apply_engine::{ApplyInput, apply_pair};
use crate::compare::engine::CancelFlag;
use crate::compare::export::plan::destination_side;
use crate::compare::key::compare_tuples;
use crate::compare::protocol::{BulkApply, CompareEvent, PairId, PairSpec, RunId, SessionId, Side};
use crate::compare::results::{Category, ResultSet};
use crate::error::{CoreError, Result};
use crate::store::{CollectionKind, DocumentStore, Namespace};

use super::runner::RunSlot;
use super::session::{ActiveRun, Session};
use super::{Events, Repaint, Shared, answer_closed, bounded, connections};

/// What an `Apply` command asks for.
pub(super) struct ApplyRequest {
    pub ops: Vec<ApplyOp>,
    pub bulk: Vec<BulkApply>,
    pub id_policy: IdPolicy,
}

/// The operations of one pair, in request order.
pub(super) type Group = (PairId, Vec<ApplyOp>);

/// Everything one apply (or export) needs to report back.
#[derive(Clone)]
pub(super) struct JobContext {
    pub(super) session: Arc<Session>,
    pub(super) events: Events,
    pub(super) repaint: Repaint,
    pub(super) session_id: SessionId,
    pub(super) run: RunId,
    /// Bound for the destination listing calls.
    pub(super) limit: Duration,
}

impl JobContext {
    /// Sends `event` unless the session is closed.
    pub(super) fn emit(&self, event: CompareEvent) -> bool {
        self.session
            .emit_if_open(&self.events, &self.repaint, event)
    }
}

pub(super) fn internal_error() -> CoreError {
    CoreError::Database("internal error".into())
}

pub(super) fn dispatch(
    shared: &Shared,
    events: Events,
    repaint: Repaint,
    id: SessionId,
    run: RunId,
    request: ApplyRequest,
) {
    let Some(session) = shared.compare.get(id).filter(|s| s.is_open()) else {
        return answer_closed(&events, &repaint, id, "compare_apply");
    };
    let ctx = JobContext {
        session,
        events,
        repaint,
        session_id: id,
        run,
        limit: shared.config.command_timeout,
    };
    if let Err(error) = start_apply(ctx.clone(), request) {
        ctx.emit(CompareEvent::Failed {
            session: id,
            context: "compare_apply",
            error,
        });
    }
}

/// The synchronous part: validates, takes the job slot, spawns the task.
fn start_apply(ctx: JobContext, request: ApplyRequest) -> Result<()> {
    connections(&ctx.session)?;
    let ApplyRequest {
        mut ops,
        bulk,
        id_policy: policy,
    } = request;
    ops.extend(expand_bulk(&ctx.session, &bulk)?);
    if ops.is_empty() {
        return Err(CoreError::Unsupported("nothing to apply".into()));
    }
    refuse_misshapen(&ops)?;
    refuse_duplicates(&ops)?;
    let groups = group_by_pair(ops);
    require_results(&ctx.session, &groups)?;
    let cancel = CancelFlag::default();
    let slot = take_slot(&ctx.session, ctx.run, &cancel)?;
    tokio::spawn(run_apply_contained(ctx, groups, policy, cancel, slot));
    Ok(())
}

/// One operation per stored row of each bulk item's category, in stored order. Each pair's
/// results are read through a cloned `Arc`, so the pairs lock is not held while keys are copied.
/// A repeated `(pair, category)` is refused even when it would expand to nothing.
pub(super) fn expand_bulk(session: &Session, bulk: &[BulkApply]) -> Result<Vec<ApplyOp>> {
    refuse_repeated_categories(bulk)?;
    let mut ops = Vec::new();
    for item in bulk {
        let results = snapshot(session, item.pair)
            .map(|(_, results)| results)
            .ok_or_else(|| CoreError::Unsupported("no results for this pair".into()))?;
        ops.extend(bulk_to_ops(item, &results)?);
    }
    Ok(ops)
}

fn refuse_repeated_categories(bulk: &[BulkApply]) -> Result<()> {
    let repeated = bulk.iter().enumerate().any(|(i, a)| {
        bulk[..i]
            .iter()
            .any(|b| b.pair == a.pair && b.category == a.category)
    });
    if repeated {
        return Err(CoreError::Unsupported("duplicate operation".into()));
    }
    Ok(())
}

/// Which sides a row of `category` exists on: `(has_source, has_target)`.
fn category_sides(category: Category) -> (bool, bool) {
    match category {
        Category::OnlyInSource => (true, false),
        Category::OnlyInTarget => (false, true),
        Category::Different => (true, true),
    }
}

/// The operations of one bulk item. The kind must fit the category (checked even when the
/// category is empty, so the request is judged by what it asks for, not by what is stored).
fn bulk_to_ops(item: &BulkApply, results: &ResultSet) -> Result<Vec<ApplyOp>> {
    let (has_source, has_target) = category_sides(item.category);
    // Field copies are chosen per document; there is no category-wide field copy.
    if item.kind == ApplyKind::Fields
        || !kind_fits(item.kind, item.direction, has_source, has_target)
    {
        return Err(CoreError::Unsupported(
            "operation does not fit this category".into(),
        ));
    }
    let all = results.page(item.category, 0, results.count(item.category));
    Ok(all
        .iter()
        .map(|entry| ApplyOp {
            pair: item.pair,
            key: entry.key.clone(),
            kind: item.kind,
            direction: item.direction,
            fields: Vec::new(),
        })
        .collect())
}

/// A `Fields` op names at least one path; every other kind names none. Judged before anything
/// else about the request, so a malformed op never reaches the slot or a store.
pub(super) fn refuse_misshapen(ops: &[ApplyOp]) -> Result<()> {
    let misshapen = ops
        .iter()
        .any(|op| (op.kind == ApplyKind::Fields) == op.fields.is_empty());
    if misshapen {
        return Err(CoreError::Unsupported(
            "operation does not fit this category".into(),
        ));
    }
    Ok(())
}

/// No two operations may act on the same `(pair, key)`; key equality is `compare_tuples`.
pub(super) fn refuse_duplicates(ops: &[ApplyOp]) -> Result<()> {
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
pub(super) fn group_by_pair(ops: Vec<ApplyOp>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for op in ops {
        match groups.iter_mut().find(|(pair, _)| *pair == op.pair) {
            Some((_, list)) => list.push(op),
            None => groups.push((op.pair, vec![op])),
        }
    }
    groups
}

pub(super) fn require_results(session: &Session, groups: &[Group]) -> Result<()> {
    let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
    if groups.iter().all(|(pair, _)| pairs.contains_key(pair)) {
        return Ok(());
    }
    Err(CoreError::Unsupported("no results for this pair".into()))
}

/// Takes the one job slot of the session (the same slot a compare run uses).
pub(super) fn take_slot(
    session: &Arc<Session>,
    run: RunId,
    cancel: &CancelFlag,
) -> Result<RunSlot> {
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
    ctx: JobContext,
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
    ctx: JobContext,
    groups: Vec<Group>,
    policy: IdPolicy,
    cancel: CancelFlag,
    slot: RunSlot,
) {
    if let Err(error) = check_destinations(&ctx.session, ctx.limit, &groups).await {
        // Freed first: a command sent on seeing the `Failed` must find the slot free.
        slot.release();
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

pub(super) fn destination_namespace(spec: &PairSpec, side: Side) -> &Namespace {
    match side {
        Side::Source => &spec.source,
        Side::Target => &spec.target,
    }
}

/// Before any write: no destination may be a view, and a time-series destination takes inserts
/// only (so not field updates either). A collection that does not exist yet is fine (the server creates it on insert).
pub(super) async fn check_destinations(
    session: &Arc<Session>,
    limit: Duration,
    groups: &[Group],
) -> Result<()> {
    let writes = planned_writes(session, groups);
    // Its own task: a panicking store ends in a refusal like any other, not in a bare `ApplyDone`.
    tokio::spawn(check_writes(session.clone(), limit, writes))
        .await
        .unwrap_or_else(|_| Err(internal_error()))
}

async fn check_writes(session: Arc<Session>, limit: Duration, writes: Writes) -> Result<()> {
    let conns = connections(&session)?;
    for ((side, db), wanted) in writes {
        let store = conns.store(side);
        let kinds = bounded(limit, async { store.list_collection_kinds(&db).await }).await?;
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
pub(super) fn snapshot(session: &Session, pair: PairId) -> Option<(PairSpec, Arc<ResultSet>)> {
    let pairs = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
    pairs
        .get(&pair)
        .map(|stored| (stored.spec.clone(), stored.results.clone()))
}

async fn apply_group(
    ctx: &JobContext,
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
        Err(_) => {
            // A panic stops the whole apply: later pairs are reported `not_run`. Writes of this
            // pair may have happened, so its ops are failed as outcome unknown.
            cancel.cancel();
            failed_report(&ops, &CoreError::ApplyOutcomeUnknown)
        }
    }
}

async fn run_engine(
    ctx: &JobContext,
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
