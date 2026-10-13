//! Exports the pending changes of compare pairs to a file as a mongosh script or a JSON plan.
//!
//! Like `apply::dispatch`, `dispatch` validates synchronously (session open and connected, the
//! request valid, every pair has results, the path usable, slot free) so command ORDER decides
//! outcomes; the destination checks, the planning and the writing run in a task. The planning is
//! the apply planner's (`plan_pair_export`): documents are only READ, never written.
//!
//! Answers: `ExportProgress`* then exactly one `ExportDone`, unless the session was closed (then
//! nothing more is sent). **For the UI:** a refusal found after the slot was taken (the
//! destination rules) and a failure to create or write the file are a
//! `Failed { context: "compare_export" }` WITHOUT `ExportDone`; it is the end of the export, sent
//! after the slot was freed and the partial file removed. Its error is `ExportFileNotRemoved`
//! when the removal itself failed (the person must delete the file); otherwise no file is left.
//! In `ExportDone` of a cancelled export (or one that panicked) the file is gone and the counts
//! say nothing was exported (`not_run` = all operations); `partial_removed` is false only when no
//! file existed yet or its removal failed.
//!
//! Scripts have ONE destination side: a request whose operations write to both sides is refused
//! for a script (`Unsupported("export one direction at a time")`); a JSON plan may mix them.
//! The pairs are read once, when the request is accepted: a pair discarded afterwards is still
//! exported from the results the person saw.
//!
//! File lifecycle: the file is created at the chosen path (owner-only, never following a
//! symlink), no temp file is made and nothing is renamed. It starts as INCOMPLETE (a script
//! whose first line is `quit(1);`, a plan with `"complete":false`) with a header of fixed
//! length; only after the body was written and synced is the header overwritten IN PLACE by the
//! final one (true counts, "complete"), so a file cut short by a crash cannot run or pass as
//! whole. It stays only if the export completed, was not cancelled and the session is still open
//! and heard `ExportDone`; in every other case (cancel, close, error, panic) the partial file is
//! removed (`FileGuard`).
//!
//! File IO: the file is written with std buffered IO from the planner's batch sink (at most 500
//! operations rendered per step, 64 KiB buffer, so memory is bounded); each such write is short
//! and the waiting (document reads) is async, so it runs on the runtime thread instead of being
//! shipped to a blocking thread per batch. Creating the file and the final sync + header
//! overwrite run in `spawn_blocking`.
//!
//! Keys, documents and paths are data and are never logged.

use std::path::PathBuf;
use std::sync::Arc;

use crate::compare::apply::{ApplyOp, IdPolicy};
use crate::compare::apply_engine::ApplyInput;
use crate::compare::engine::CancelFlag;
use crate::compare::export::plan::{ExportItem, destination_side, plan_pair_export};
use crate::compare::export::{ExportCounts, ExportHeader, count_width};
use crate::compare::protocol::{
    CompareEvent, ExportFormat, ExportOutcome, ExportRequest, RunId, SessionId, Side,
};
use crate::error::{CoreError, Result};
use crate::store::DocumentStore;

use super::apply::{
    Group, JobContext, check_destinations, expand_bulk, group_by_pair, internal_error,
    refuse_duplicates, refuse_misshapen, require_results, snapshot, take_slot,
};
use super::export_file::{FileGuard, overwrite_header, validate_path};
use super::export_sink::{
    ExportSink, FormatWriter, Resolved, Section, Tally, Unit, header_bytes, render_error, sections,
};
use super::runner::RunSlot;
use super::{Events, Repaint, Shared, answer_closed, connections};

/// What the task needs besides the context.
struct ExportJob {
    format: ExportFormat,
    label: String,
    policy: IdPolicy,
    resolved: Vec<Resolved>,
    /// The one side a script writes to (`None` for a plan).
    destination: Option<Side>,
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
    let destination = script_destination(format, &ops)?;
    let groups = group_by_pair(ops);
    require_results(&ctx.session, &groups)?;
    validate_path(&path)?;
    let resolved = resolve(&ctx.session, groups)?;
    let cancel = CancelFlag::default();
    let slot = take_slot(&ctx.session, ctx.run, &cancel)?;
    let job = ExportJob {
        format,
        label,
        policy,
        resolved,
        destination,
        path,
        cancel,
    };
    tokio::spawn(run_export(ctx, job, slot));
    Ok(())
}

/// A script writes to one side only (its statements name db and collection but not a server);
/// a plan carries the direction of every operation. `None` for a plan.
fn script_destination(format: ExportFormat, ops: &[ApplyOp]) -> Result<Option<Side>> {
    if format == ExportFormat::JsonPlan {
        return Ok(None);
    }
    let first = ops.first().map(destination_side);
    if ops.iter().any(|op| Some(destination_side(op)) != first) {
        return Err(CoreError::Unsupported(
            "export one direction at a time".into(),
        ));
    }
    Ok(first)
}

/// The pairs of the request with their ops, read once now.
fn resolve(session: &super::session::Session, groups: Vec<Group>) -> Result<Vec<Resolved>> {
    groups
        .into_iter()
        .map(|(pair, ops)| {
            let (spec, results) = snapshot(session, pair).ok_or(CoreError::NotFound)?;
            Ok(Resolved { spec, results, ops })
        })
        .collect()
}

fn groups_of(resolved: &[Resolved]) -> Vec<Group> {
    resolved
        .iter()
        .map(|r| (r.spec.id, r.ops.clone()))
        .collect()
}

/// An outcome in which nothing was exported: the file is gone (or never existed).
fn nothing_exported(total: u64, cancelled: bool, partial_removed: bool) -> ExportOutcome {
    ExportOutcome {
        not_run: total,
        cancelled,
        partial_removed,
        ..ExportOutcome::default()
    }
}

/// Checks the destinations, creates the file and runs the writing in an inner task, so a panic
/// there still frees the slot, removes the file and ends with an `ExportDone`.
async fn run_export(ctx: JobContext, job: ExportJob, slot: RunSlot) {
    let total: u64 = job.resolved.iter().map(|r| r.ops.len() as u64).sum();
    if let Err(error) = check_destinations(&ctx.session, ctx.limit, &groups_of(&job.resolved)).await
    {
        // Freed first: a command sent on seeing the `Failed` must find the slot free.
        slot.release();
        ctx.emit(failed(&ctx, error));
        return;
    }
    if job.cancel.is_cancelled() {
        slot.release();
        ctx.emit(done(&ctx, nothing_exported(total, true, false)));
        return;
    }
    let path = job.path.clone();
    let created = tokio::task::spawn_blocking(move || FileGuard::create(&path)).await;
    let (file, guard) = match created {
        Ok(Ok(made)) => made,
        Ok(Err(error)) => return refuse(&ctx, slot, error),
        Err(_) => return refuse(&ctx, slot, internal_error()),
    };
    let cancel = job.cancel.clone();
    let sink = ExportSink::new(file, &job.path);
    let joined = tokio::spawn(write_export(ctx.clone(), job, sink, total)).await;
    conclude(&ctx, slot, guard, &cancel, total, joined);
}

fn refuse(ctx: &JobContext, slot: RunSlot, error: CoreError) {
    slot.release();
    ctx.emit(failed(ctx, error));
}

type Joined = std::result::Result<Result<ExportOutcome>, tokio::task::JoinError>;

/// Decides whether the file stays, then frees the slot and reports. The file stays only when the
/// export ran to the end, was not cancelled, and `ExportDone` reached an open session. A failed
/// removal is reported (`partial_removed: false`, or `ExportFileNotRemoved` for a failure).
fn conclude(
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
            if ctx.emit(done(ctx, outcome)) {
                guard.commit();
            } else {
                // Closed meanwhile: nobody can be told, the file must still go.
                guard.remove();
            }
        }
        Ok(Err(error)) => {
            let removed = guard.remove();
            slot.release();
            let error = if removed {
                error
            } else {
                CoreError::ExportFileNotRemoved
            };
            ctx.emit(failed(ctx, error));
        }
        // Cancelled, closed or panicked: the file goes and nothing counts as exported.
        _ => {
            let removed = guard.remove();
            slot.release();
            ctx.emit(done(ctx, nothing_exported(total, cancelled, removed)));
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

/// The header of an export of `total` operations: counts padded to a fixed width, marked
/// incomplete until the end.
fn base_header(job: &ExportJob, total: u64) -> ExportHeader {
    ExportHeader {
        label: job.label.clone(),
        counts: ExportCounts::default(),
        id_policy: job.policy,
        count_width: count_width(total),
        complete: false,
        destination: job.destination,
    }
}

/// Plans and writes everything; returns the outcome counts (not yet judged for cancel/close).
async fn write_export(
    ctx: JobContext,
    job: ExportJob,
    sink: ExportSink,
    total: u64,
) -> Result<ExportOutcome> {
    let base = base_header(&job, total);
    let first_len = header_bytes(job.format, &base).map_err(render_error)?.len() as u64;
    let mut writer = FormatWriter::new(job.format, sink);
    writer.begin(&base).map_err(render_error)?;
    let conns = connections(&ctx.session)?;
    let mut run = Run {
        ctx: &ctx,
        stores: (conns.source.clone(), conns.target.clone()),
        policy: job.policy,
        cancel: &job.cancel,
        total,
        done: 0,
        not_run: 0,
        tally: Tally::default(),
        writer,
    };
    for section in sections(job.format, &job.resolved) {
        run.section(&job.resolved, section).await?;
    }
    finish(run, (job.format, base), first_len).await
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

/// Closes the document, syncs it and overwrites the incomplete header by the final one (same
/// length, one small write). Skipped when cancelled: the file is about to be removed.
async fn finish(
    run: Run<'_>,
    (format, base): (ExportFormat, ExportHeader),
    first_len: u64,
) -> Result<ExportOutcome> {
    let Run {
        writer,
        tally,
        not_run,
        cancel,
        ..
    } = run;
    if cancel.is_cancelled() {
        return Ok(ExportOutcome::default());
    }
    let file = writer.finish().map_err(render_error)?;
    let header = ExportHeader {
        counts: tally.counts,
        complete: true,
        ..base
    };
    let bytes = header_bytes(format, &header).map_err(render_error)?;
    let length = tokio::task::spawn_blocking(move || {
        let mut file = file;
        overwrite_header(&mut file, first_len, &bytes)?;
        file.metadata().map(|m| m.len())
    })
    .await
    .map_err(|_| internal_error())?
    .map_err(|_| CoreError::ExportWriteFailed)?;
    Ok(ExportOutcome {
        written: tally.written(),
        skipped: tally.skipped(),
        failed: tally.unreadable,
        not_run,
        bytes: length,
        cancelled: false,
        partial_removed: false,
    })
}
