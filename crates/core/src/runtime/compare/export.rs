//! Exports the pending changes of compare pairs to a file as a mongosh script or a JSON plan.
//!
//! Like `apply::dispatch`, `dispatch` validates synchronously (session open and connected, the
//! request valid, every pair has results, the path usable, slot free) so command ORDER decides
//! outcomes; the destination checks, the planning and the writing run in a task. The planning is
//! the apply planner's (`plan_pair_export`): documents are only READ, never written.
//!
//! Answers: `ExportProgress`* then exactly one `ExportDone`, unless the session was closed (then
//! nothing more is sent). A refusal found after the slot was taken (the destination rules) and a
//! failure to create or write the file are a `Failed { context: "compare_export" }` without
//! `ExportDone`, sent after the slot was freed and the file removed.
//!
//! File lifecycle: the file is created at the chosen path (truncating, owner-only), no temp file
//! is made and nothing is renamed. It stays only if the export completed, was not cancelled and
//! the session is still open and heard `ExportDone`; in every other case (cancel, close, error,
//! panic) the partial file is removed (`FileGuard`) and `partial_removed` says so.
//!
//! File IO: the file is written with std buffered IO from the planner's batch sink (at most 500
//! operations rendered per step, 64 KiB buffer, so memory is bounded); each such write is short
//! and the waiting (document reads) is async, so it runs on the runtime thread instead of being
//! shipped to a blocking thread per batch. Creating the file and patching the header at the end
//! (which may rewrite the whole file once) run in `spawn_blocking`.
//!
//! Keys, documents and paths are data and are never logged.

use std::path::PathBuf;
use std::sync::Arc;

use crate::compare::apply::{ApplyOp, IdPolicy};
use crate::compare::apply_engine::ApplyInput;
use crate::compare::engine::CancelFlag;
use crate::compare::export::plan::{ExportItem, plan_pair_export};
use crate::compare::export::{ExportCounts, ExportHeader};
use crate::compare::protocol::{
    CompareEvent, ExportFormat, ExportOutcome, ExportRequest, RunId, SessionId,
};
use crate::error::{CoreError, Result};
use crate::store::DocumentStore;

use super::apply::{
    JobContext, check_destinations, expand_bulk, group_by_pair, internal_error, refuse_duplicates,
    refuse_misshapen, require_results, snapshot, take_slot,
};
use super::export_file::{FileGuard, replace_header, validate_path};
use super::export_sink::{
    FormatWriter, Resolved, Section, Tally, Unit, header_bytes, render_error, sections,
};
use super::runner::RunSlot;
use super::{Events, Repaint, Shared, answer_closed, connections};

/// What the task needs besides the context.
struct ExportJob {
    format: ExportFormat,
    label: String,
    policy: IdPolicy,
    groups: Vec<(u32, Vec<ApplyOp>)>,
    path: PathBuf,
    cancel: CancelFlag,
}

pub(super) fn dispatch(
    shared: &Shared,
    events: Events,
    repaint: Repaint,
    id: SessionId,
    run: RunId,
    (request, path): (ExportRequest, PathBuf),
) {
    let Some(session) = shared.compare.get(id).filter(|s| s.is_open()) else {
        return answer_closed(&events, &repaint, id, "compare_export");
    };
    let ctx = JobContext {
        session,
        events,
        repaint,
        session_id: id,
        run,
        limit: shared.config.command_timeout,
    };
    if let Err(error) = start_export(ctx.clone(), request, path) {
        ctx.emit(failed(&ctx, error));
    }
}

fn failed(ctx: &JobContext, error: CoreError) -> CompareEvent {
    CompareEvent::Failed {
        session: ctx.session_id,
        context: "compare_export",
        error,
    }
}

/// The synchronous part: validates, takes the job slot, spawns the task.
fn start_export(ctx: JobContext, request: ExportRequest, path: PathBuf) -> Result<()> {
    connections(&ctx.session)?;
    let ExportRequest {
        format,
        label,
        id_policy: policy,
        mut ops,
        bulk,
    } = request;
    ops.extend(expand_bulk(&ctx.session, &bulk)?);
    if ops.is_empty() {
        return Err(CoreError::Unsupported("nothing to export".into()));
    }
    refuse_misshapen(&ops)?;
    refuse_duplicates(&ops)?;
    let groups = group_by_pair(ops);
    require_results(&ctx.session, &groups)?;
    validate_path(&path)?;
    let cancel = CancelFlag::default();
    let slot = take_slot(&ctx.session, ctx.run, &cancel)?;
    let job = ExportJob {
        format,
        label,
        policy,
        groups,
        path,
        cancel,
    };
    tokio::spawn(run_export(ctx, job, slot));
    Ok(())
}

/// Checks the destinations, creates the file and runs the writing in an inner task, so a panic
/// there still frees the slot, removes the file and ends with an `ExportDone`.
async fn run_export(ctx: JobContext, job: ExportJob, slot: RunSlot) {
    if let Err(error) = check_destinations(&ctx.session, ctx.limit, &job.groups).await {
        // Freed first: a command sent on seeing the `Failed` must find the slot free.
        slot.release();
        ctx.emit(failed(&ctx, error));
        return;
    }
    let path = job.path.clone();
    let created = tokio::task::spawn_blocking(move || FileGuard::create(&path)).await;
    let (file, guard) = match created {
        Ok(Ok(made)) => made,
        Ok(Err(error)) => return refuse(&ctx, slot, error),
        Err(_) => return refuse(&ctx, slot, internal_error()),
    };
    let total: u64 = job.groups.iter().map(|(_, ops)| ops.len() as u64).sum();
    let cancel = job.cancel.clone();
    let joined = tokio::spawn(write_export(ctx.clone(), job, file, total)).await;
    conclude(&ctx, slot, guard, &cancel, total, joined).await;
}

fn refuse(ctx: &JobContext, slot: RunSlot, error: CoreError) {
    slot.release();
    ctx.emit(failed(ctx, error));
}

type Joined = std::result::Result<Result<ExportOutcome>, tokio::task::JoinError>;

/// Decides whether the file stays, then frees the slot and reports. The file stays only when the
/// export ran to the end, was not cancelled, and `ExportDone` reached an open session.
async fn conclude(
    ctx: &JobContext,
    slot: RunSlot,
    guard: FileGuard,
    cancel: &CancelFlag,
    total: u64,
    joined: Joined,
) {
    let cancelled = cancel.is_cancelled();
    match joined {
        Ok(Ok(outcome)) if !cancelled && ctx.session.is_open() => {
            slot.release();
            let sent = ctx.emit(done(ctx, outcome));
            if sent {
                guard.commit();
            } else {
                guard.remove();
            }
        }
        Ok(Ok(outcome)) => {
            let removed = guard.remove();
            slot.release();
            let outcome = ExportOutcome {
                cancelled,
                partial_removed: removed,
                bytes: 0,
                ..outcome
            };
            ctx.emit(done(ctx, outcome));
        }
        Ok(Err(error)) => {
            guard.remove();
            slot.release();
            ctx.emit(failed(ctx, error));
        }
        Err(_) => {
            let removed = guard.remove();
            slot.release();
            let outcome = ExportOutcome {
                not_run: total,
                cancelled,
                partial_removed: removed,
                ..ExportOutcome::default()
            };
            ctx.emit(done(ctx, outcome));
        }
    }
}

fn done(ctx: &JobContext, outcome: ExportOutcome) -> CompareEvent {
    CompareEvent::ExportDone {
        session: ctx.session_id,
        run: ctx.run,
        outcome,
    }
}

/// The pairs of the request as they are now. A pair discarded since the request cannot be
/// planned: its ops count as failed (apply reports them as `NotFound`).
fn resolve(ctx: &JobContext, groups: Vec<(u32, Vec<ApplyOp>)>) -> (Vec<Resolved>, u64) {
    let mut resolved = Vec::new();
    let mut missing = 0u64;
    for (pair, ops) in groups {
        match snapshot(&ctx.session, pair) {
            Some((spec, results)) => resolved.push(Resolved { spec, results, ops }),
            None => missing += ops.len() as u64,
        }
    }
    (resolved, missing)
}

/// The header with every count an upper bound of the true one, so the true header is never
/// longer; it is replaced by the true one at the end.
fn provisional_header(base: &ExportHeader, total: u64) -> ExportHeader {
    ExportHeader {
        counts: ExportCounts {
            insert: total,
            replace: total,
            delete: total,
            update_fields: total,
            skipped: total,
        },
        ..base.clone()
    }
}

/// Plans and writes everything; returns the outcome counts (not yet judged for cancel/close).
async fn write_export(
    ctx: JobContext,
    job: ExportJob,
    file: std::fs::File,
    total: u64,
) -> Result<ExportOutcome> {
    let base = ExportHeader {
        label: job.label,
        counts: ExportCounts::default(),
        id_policy: job.policy,
    };
    let provisional = provisional_header(&base, total);
    let first = header_bytes(job.format, &provisional).map_err(render_error)?;
    let mut writer = FormatWriter::new(job.format, file);
    writer.begin(&provisional).map_err(render_error)?;
    let (resolved, missing) = resolve(&ctx, job.groups);
    let conns = connections(&ctx.session)?;
    let mut run = Run {
        ctx: &ctx,
        stores: (conns.source.clone(), conns.target.clone()),
        policy: job.policy,
        cancel: &job.cancel,
        total,
        done: missing,
        not_run: 0,
        tally: Tally {
            failed: missing,
            ..Tally::default()
        },
        writer,
    };
    for section in sections(job.format, &resolved) {
        run.section(&resolved, section).await?;
    }
    finish(run, (job.format, base), first.len() as u64).await
}

/// The state of the writing.
struct Run<'a> {
    ctx: &'a JobContext,
    stores: (Arc<dyn DocumentStore>, Arc<dyn DocumentStore>),
    policy: IdPolicy,
    cancel: &'a CancelFlag,
    total: u64,
    done: u64,
    not_run: u64,
    tally: Tally,
    writer: FormatWriter,
}

impl Run<'_> {
    async fn section(&mut self, resolved: &[Resolved], section: Section) -> Result<()> {
        self.writer.open(&section.head).map_err(render_error)?;
        for unit in section.units {
            self.unit(&resolved[unit.resolved], unit).await?;
        }
        self.writer.close().map_err(render_error)
    }

    /// Plans one unit into the writer. The sink writes each batch as it is planned.
    async fn unit(&mut self, pair: &Resolved, unit: Unit) -> Result<()> {
        let (ctx, base, total) = (self.ctx, self.done, self.total);
        let input = ApplyInput {
            spec: &pair.spec,
            results: &pair.results,
            source: self.stores.0.clone(),
            target: self.stores.1.clone(),
            ops: &unit.ops,
            policy: self.policy,
        };
        let (writer, tally) = (&mut self.writer, &mut self.tally);
        let mut sink = |items: Vec<ExportItem>| -> Result<()> {
            for item in items {
                *tally = tally.with(&item);
                writer.item(item).map_err(render_error)?;
            }
            Ok(())
        };
        let mut progress = |in_unit: u64| {
            ctx.emit(CompareEvent::ExportProgress {
                session: ctx.session_id,
                run: ctx.run,
                done: base + in_unit,
                total,
            });
        };
        let counts = plan_pair_export(input, self.cancel, &mut sink, &mut progress).await?;
        self.not_run += counts.not_run;
        self.done += unit.ops.len() as u64 - counts.not_run;
        Ok(())
    }
}

/// Closes the document, writes the true header over the provisional one and sizes the outcome.
async fn finish(
    run: Run<'_>,
    (format, base): (ExportFormat, ExportHeader),
    provisional_len: u64,
) -> Result<ExportOutcome> {
    let Run {
        writer,
        tally,
        not_run,
        ..
    } = run;
    let file = writer.finish().map_err(render_error)?;
    let header = ExportHeader {
        counts: tally.counts,
        ..base
    };
    let bytes = header_bytes(format, &header).map_err(render_error)?;
    let length = tokio::task::spawn_blocking(move || {
        let mut file = file;
        replace_header(&mut file, provisional_len, &bytes)
    })
    .await
    .map_err(|_| internal_error())?
    .map_err(|_| CoreError::ExportWriteFailed)?;
    Ok(ExportOutcome {
        written: tally.written(),
        skipped: tally.counts.skipped,
        failed: tally.failed,
        not_run,
        bytes: length,
        cancelled: false,
        partial_removed: false,
    })
}
