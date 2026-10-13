//! Exports a CSV report of the stored differences of compare pairs, to a file or as text for the
//! clipboard.
//!
//! Like `export::dispatch`, `dispatch` validates synchronously (session open and connected, at
//! least one pair and none twice, every pair has results, the clipboard report within
//! [`CLIPBOARD_MAX_ROWS`], the file path usable, slot free) so command ORDER decides outcomes;
//! everything else runs in a task. The pairs are read once, when the request is accepted: a pair
//! discarded afterwards is still reported from the results the person saw.
//!
//! Answers: `ExportProgress`*, for the clipboard one `ExportText` (the whole report), then
//! exactly one `ExportDone` (rows in `written`), unless the session was closed (then nothing
//! more is sent). A refusal, and a failure to create or write the file, are a
//! `Failed { context: "compare_export" }` WITHOUT `ExportDone`, as for a plan export. In the
//! `ExportDone` of a cancelled report (or one that panicked) nothing was exported (`not_run` =
//! all rows), no text was sent and no file remains.
//!
//! Accounting (different from a plan export): `written` is the rows written. With
//! `include_paths`, a different row whose documents changed or vanished since the compare (or
//! are no longer unique, or whose fresh diff finds nothing) gets an empty `changed_paths` cell
//! and counts in `skipped`; one whose documents could not be read counts in `failed`. Both are
//! counted INSIDE `written`: those rows are still listed.
//!
//! The summary needs no header overwrite: the per-pair counts are known from the stored results
//! before the first row, so the file is written once, front to back. It has no "incomplete"
//! marker; a file cut short by a crash is a valid CSV with fewer rows, which is why the file is
//! removed on every failure path and only kept after a clean end (`FileGuard`), exactly as for a
//! plan export (see `export.rs` for the lifecycle). Memory is bounded: rows are written one at a
//! time through a 64 KiB buffer (the clipboard text, at most [`CLIPBOARD_MAX_ROWS`] rows, is
//! built in memory).
//!
//! The clipboard cap counts rows. With `include_paths` one row can carry many paths, so the
//! `changed_paths` cell is cut at 200 paths plus a `"..."` element: the size stays bounded.
//!
//! Format notes: the `pair` column is the 1-based position of the pair in the report (it matches
//! `# pair N` in the summary), not the pair id. Display paths are ambiguous for field names that
//! contain `.` or `[`. `changed_paths` is a JSON array of display paths; this replaces the
//! `; `-joined form of the brief (an intentional deviation: a JSON array survives names that
//! contain `;`). The last line `# end of report: N rows` marks a complete file.
//!
//! Keys, names and paths are data and are never logged.

use std::io::Write;
use std::sync::Arc;

use crate::compare::diff::PathSegment;
use crate::compare::engine::{CancelFlag, Summary};
use crate::compare::export::csv::{CsvRow, CsvWriter};
use crate::compare::export::report::{
    CLIPBOARD_MAX_ROWS, ChangedPaths, REPORT_CATEGORIES, ROW_BATCH, changed_paths, display_name,
    pair_summary, report_header, row_count,
};
use crate::compare::protocol::{
    CompareEvent, ExportOutcome, PairId, PairSpec, ReportRequest, ReportTarget, RunId, SessionId,
};
use crate::compare::results::{Category, DiffEntry, ResultSet};
use crate::compare::scan::{Unique, fetch_unique};
use crate::error::{CoreError, Result};
use crate::store::DocumentStore;

use super::apply::{JobContext, internal_error, take_slot};
use super::export::{done, failed, nothing_exported, refuse};
use super::export_file::{FileGuard, validate_path};
use super::export_sink::{ExportSink, buffered, into_file, render_error};
use super::runner::RunSlot;
use super::session::Session;
use super::{Events, Repaint, Shared, answer_closed, connections};

/// Rows between two progress events while documents are read (each row is slow then).
const PATH_PROGRESS: u64 = 20;

/// One pair of the request as it is now.
struct ReportPair {
    spec: PairSpec,
    results: Arc<ResultSet>,
    reported: Summary,
}

struct ReportJob {
    pairs: Vec<ReportPair>,
    include_paths: bool,
    label: String,
    target: ReportTarget,
    total: u64,
    cancel: CancelFlag,
}

pub(super) fn dispatch(
    shared: &Shared,
    events: Events,
    repaint: Repaint,
    id: SessionId,
    run: RunId,
    (request, target): (ReportRequest, ReportTarget),
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
    if let Err(error) = start_report(ctx.clone(), request, target) {
        ctx.emit(failed(&ctx, error));
    }
}

/// The synchronous part: validates, takes the job slot, spawns the task.
fn start_report(ctx: JobContext, request: ReportRequest, target: ReportTarget) -> Result<()> {
    connections(&ctx.session)?;
    let pairs = resolve(&ctx.session, &request.pairs)?;
    let total = pairs
        .iter()
        .map(|p| row_count(&p.results))
        .fold(0, usize::saturating_add);
    check_target(&target, total)?;
    let cancel = CancelFlag::default();
    let slot = take_slot(&ctx.session, ctx.run, &cancel)?;
    let job = ReportJob {
        pairs,
        include_paths: request.include_paths,
        label: request.label,
        target,
        total: total as u64,
        cancel,
    };
    tokio::spawn(run_report(ctx, job, slot));
    Ok(())
}

/// The clipboard holds a bounded report; a file needs a usable path.
fn check_target(target: &ReportTarget, rows: usize) -> Result<()> {
    match target {
        ReportTarget::Clipboard if rows > CLIPBOARD_MAX_ROWS => Err(CoreError::Unsupported(
            "report too large for the clipboard".into(),
        )),
        ReportTarget::Clipboard => Ok(()),
        ReportTarget::File(path) => validate_path(path),
    }
}

/// The pairs of the request in request order, read once now.
fn resolve(session: &Session, ids: &[PairId]) -> Result<Vec<ReportPair>> {
    if ids.is_empty() {
        return Err(CoreError::Unsupported("nothing to export".into()));
    }
    if ids.iter().enumerate().any(|(i, id)| ids[..i].contains(id)) {
        return Err(CoreError::Unsupported("duplicate pair".into()));
    }
    let stored = session.pairs.lock().unwrap_or_else(|e| e.into_inner());
    ids.iter()
        .map(|id| {
            let pair = stored
                .get(id)
                .ok_or_else(|| CoreError::Unsupported("no results for this pair".into()))?;
            Ok(ReportPair {
                spec: pair.spec.clone(),
                results: pair.results.clone(),
                reported: pair.summary,
            })
        })
        .collect()
}

/// What a finished write hands to the conclusion.
struct Finished {
    outcome: ExportOutcome,
    /// The clipboard report (`None` for a file).
    text: Option<String>,
}

type Joined = std::result::Result<Result<Finished>, tokio::task::JoinError>;

/// Runs the writing in an inner task, so a panic there still frees the slot, removes the file
/// and ends with an `ExportDone`.
async fn run_report(ctx: JobContext, job: ReportJob, slot: RunSlot) {
    let (total, cancel) = (job.total, job.cancel.clone());
    if cancel.is_cancelled() {
        slot.release();
        ctx.emit(done(&ctx, nothing_exported(total, true, true)));
        return;
    }
    let (guard, joined) = match job.target.clone() {
        ReportTarget::File(path) => match create_file(&path).await {
            Ok((sink, guard)) => {
                let joined = tokio::spawn(write_file(ctx.clone(), job, sink)).await;
                (Some(guard), joined)
            }
            Err(error) => return refuse(&ctx, slot, error),
        },
        ReportTarget::Clipboard => (None, tokio::spawn(write_text(ctx.clone(), job)).await),
    };
    conclude(&ctx, slot, guard, (&cancel, total), joined);
}

async fn create_file(path: &std::path::Path) -> Result<(ExportSink, FileGuard)> {
    let owned = path.to_path_buf();
    let created = tokio::task::spawn_blocking(move || FileGuard::create(&owned))
        .await
        .map_err(|_| internal_error())??;
    let (file, guard) = created;
    Ok((ExportSink::new(file, path), guard))
}

/// Decides whether the file stays, frees the slot and reports. Text and file stay only when the
/// report ran to the end, was not cancelled, and the events reached an open session.
fn conclude(
    ctx: &JobContext,
    slot: RunSlot,
    guard: Option<FileGuard>,
    (cancel, total): (&CancelFlag, u64),
    joined: Joined,
) {
    let cancelled = cancel.is_cancelled();
    match joined {
        Ok(Ok(finished)) if !cancelled && ctx.session.is_open() => {
            slot.release();
            let sent = finished.text.is_none_or(|text| emit_text(ctx, text))
                && ctx.emit(done(ctx, finished.outcome));
            settle(guard, sent);
        }
        Ok(Err(error)) => {
            let removed = guard.is_none_or(FileGuard::remove);
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
            let removed = guard.is_none_or(FileGuard::remove);
            slot.release();
            ctx.emit(done(ctx, nothing_exported(total, cancelled, removed)));
        }
    }
}

fn emit_text(ctx: &JobContext, text: String) -> bool {
    ctx.emit(CompareEvent::ExportText {
        session: ctx.session_id,
        run: ctx.run,
        text,
    })
}

/// Keeps the file when everything was heard; otherwise (closed meanwhile) it must still go.
fn settle(guard: Option<FileGuard>, heard: bool) {
    match guard {
        Some(guard) if heard => guard.commit(),
        Some(guard) => {
            guard.remove();
        }
        None => {}
    }
}

/// Counts of what was written; a new value after each row.
#[derive(Clone, Copy, Default)]
struct Tally {
    rows: u64,
    stale: u64,
    unreadable: u64,
}

impl Tally {
    fn with(self, read: &PathsRead) -> Self {
        Self {
            rows: self.rows + 1,
            stale: self.stale + u64::from(matches!(read, PathsRead::Stale)),
            unreadable: self.unreadable + u64::from(matches!(read, PathsRead::Unreadable)),
        }
    }

    fn outcome(self, bytes: u64) -> ExportOutcome {
        ExportOutcome {
            written: self.rows,
            skipped: self.stale,
            failed: self.unreadable,
            bytes,
            ..ExportOutcome::default()
        }
    }
}

/// What reading the documents of a different row gave.
enum PathsRead {
    /// No paths were asked for, or the row is not a different one.
    NotAsked,
    Found(Vec<Vec<PathSegment>>),
    Stale,
    Unreadable,
}

async fn write_file(ctx: JobContext, job: ReportJob, sink: ExportSink) -> Result<Finished> {
    let Some((out, tally)) = write_report(&ctx, &job, buffered(sink)).await? else {
        return Ok(Finished {
            outcome: ExportOutcome::default(),
            text: None,
        });
    };
    let file = into_file(out).map_err(render_error)?;
    let length = tokio::task::spawn_blocking(move || {
        file.sync_all()?;
        file.metadata().map(|m| m.len())
    })
    .await
    .map_err(|_| internal_error())?
    .map_err(|_| CoreError::ExportWriteFailed)?;
    Ok(Finished {
        outcome: tally.outcome(length),
        text: None,
    })
}

async fn write_text(ctx: JobContext, job: ReportJob) -> Result<Finished> {
    let Some((bytes, tally)) = write_report(&ctx, &job, Vec::new()).await? else {
        return Ok(Finished {
            outcome: ExportOutcome::default(),
            text: None,
        });
    };
    let length = bytes.len() as u64;
    let text = String::from_utf8(bytes).map_err(|_| internal_error())?;
    Ok(Finished {
        outcome: tally.outcome(length),
        text: Some(text),
    })
}

/// Writes the whole report into `out`; `None` when cancelled (what was written is dropped).
async fn write_report<W: Write>(
    ctx: &JobContext,
    job: &ReportJob,
    out: W,
) -> Result<Option<(W, Tally)>> {
    let summaries = job
        .pairs
        .iter()
        .map(|p| pair_summary(&p.spec, &p.results, p.reported))
        .collect();
    let mut csv = CsvWriter::new(out);
    csv.begin(&report_header(&job.label, summaries))
        .map_err(render_error)?;
    let conns = connections(&ctx.session)?;
    let mut run = Writing {
        ctx,
        job,
        stores: (conns.source.clone(), conns.target.clone()),
        csv,
        tally: Tally::default(),
    };
    for (index, pair) in job.pairs.iter().enumerate() {
        if !run.pair(index + 1, pair).await? {
            return Ok(None);
        }
    }
    let Writing { csv, tally, .. } = run;
    Ok(Some((csv.finish().map_err(render_error)?, tally)))
}

/// The state of the writing.
struct Writing<'a, W: Write> {
    ctx: &'a JobContext,
    job: &'a ReportJob,
    stores: (Arc<dyn DocumentStore>, Arc<dyn DocumentStore>),
    csv: CsvWriter<W>,
    tally: Tally,
}

impl<W: Write> Writing<'_, W> {
    /// All rows of one pair, `position` being its 1-based place in the report. False when
    /// cancelled.
    async fn pair(&mut self, position: usize, pair: &ReportPair) -> Result<bool> {
        for category in REPORT_CATEGORIES {
            let count = pair.results.count(category);
            for offset in (0..count).step_by(ROW_BATCH) {
                if self.job.cancel.is_cancelled() {
                    return Ok(false);
                }
                // Share the worker fairly: a page of rows without reads has no await.
                tokio::task::yield_now().await;
                for entry in pair.results.page(category, offset, ROW_BATCH) {
                    if !self.row(position, pair, category, entry).await? {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// One row. False when cancelled while its documents were read.
    async fn row(
        &mut self,
        position: usize,
        pair: &ReportPair,
        category: Category,
        entry: &DiffEntry,
    ) -> Result<bool> {
        let read = if self.job.include_paths && category == Category::Different {
            let reading = read_paths(&self.stores, &pair.spec, entry);
            match self.job.cancel.run_until_cancelled(reading).await {
                Some(read) => read,
                None => return Ok(false),
            }
        } else {
            PathsRead::NotAsked
        };
        let none = Vec::new();
        let paths = match &read {
            PathsRead::Found(paths) => paths,
            _ => &none,
        };
        let (source, target) = (
            display_name(&pair.spec.source),
            display_name(&pair.spec.target),
        );
        let row = CsvRow {
            pair: position,
            source: &source,
            target: &target,
            category,
            key: &entry.key,
            changed_paths: paths,
        };
        self.csv.row(&row).map_err(render_error)?;
        self.tally = self.tally.with(&read);
        self.progress(category);
        Ok(true)
    }

    fn progress(&self, category: Category) {
        let done = self.tally.rows;
        let step = if self.job.include_paths && category == Category::Different {
            PATH_PROGRESS
        } else {
            ROW_BATCH as u64
        };
        if done % step == 0 || done == self.job.total {
            self.ctx.emit(CompareEvent::ExportProgress {
                session: self.ctx.session_id,
                run: self.ctx.run,
                done,
                total: self.job.total,
            });
        }
    }
}

/// Re-reads both documents of a different row (primary reads) and diffs them. A document that
/// changed, vanished or is no longer unique makes the row stale; a read or diff error makes it
/// unreadable. Only paths come out, never values.
async fn read_paths(
    stores: &(Arc<dyn DocumentStore>, Arc<dyn DocumentStore>),
    spec: &PairSpec,
    entry: &DiffEntry,
) -> PathsRead {
    let source = fetch_unique(
        &stores.0,
        &spec.source,
        &spec.source_filter,
        &spec.key,
        &entry.key,
    );
    let Ok(source) = source.await else {
        return PathsRead::Unreadable;
    };
    let target = fetch_unique(
        &stores.1,
        &spec.target,
        &spec.target_filter,
        &spec.key,
        &entry.key,
    );
    let Ok(target) = target.await else {
        return PathsRead::Unreadable;
    };
    let (Unique::One(source), Unique::One(target)) = (source, target) else {
        return PathsRead::Stale;
    };
    match changed_paths(spec, entry, &source, &target) {
        Ok(ChangedPaths::Paths(paths)) => PathsRead::Found(paths),
        Ok(ChangedPaths::Stale) => PathsRead::Stale,
        Err(_) => PathsRead::Unreadable,
    }
}
