//! What an export writes and in which order: the sections of the file (one per target
//! collection for a script, one per pair for a plan), the writer of the chosen format, and the
//! tally behind the header counts and the outcome.

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;

use crate::compare::apply::{ApplyKind, ApplyOp};
use crate::compare::export::json_plan::{PlanOp, PlanPair, PlanWriter};
use crate::compare::export::mongosh::{ScriptWriter, ShellOp};
use crate::compare::export::plan::{ExportItem, destination_side};
use crate::compare::export::{ExportCounts, ExportHeader, OpMeta, RenderError};
use crate::compare::protocol::{ExportFormat, PairSpec, Side};
use crate::compare::results::ResultSet;
use crate::error::CoreError;
use crate::store::Namespace;

/// Buffer between the planner and the file.
const BUFFER: usize = 64 * 1024;

/// One pair of the request as it is now: its spec and results and its ops in request order.
pub(super) struct Resolved {
    pub spec: PairSpec,
    pub results: Arc<ResultSet>,
    pub ops: Vec<ApplyOp>,
}

/// Ops of one resolved pair that are planned together.
pub(super) struct Unit {
    pub resolved: usize,
    pub ops: Vec<ApplyOp>,
}

/// How a section starts in the file.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Head {
    /// A script statement on this collection of this side's server (the same name on the two
    /// sides is two collections).
    Collection(Side, Namespace),
    /// A pair entry of a plan.
    Pair(PlanPair),
}

pub(super) struct Section {
    pub head: Head,
    pub units: Vec<Unit>,
}

/// The sections of the file, in first-appearance order.
pub(super) fn sections(format: ExportFormat, resolved: &[Resolved]) -> Vec<Section> {
    match format {
        ExportFormat::JsonPlan => pair_sections(resolved),
        ExportFormat::Mongosh => collection_sections(resolved),
    }
}

fn pair_sections(resolved: &[Resolved]) -> Vec<Section> {
    resolved
        .iter()
        .enumerate()
        .map(|(i, r)| Section {
            head: Head::Pair(PlanPair {
                source: r.spec.source.clone(),
                target: r.spec.target.clone(),
                key: r.spec.key.fields.clone(),
            }),
            units: vec![Unit {
                resolved: i,
                ops: r.ops.clone(),
            }],
        })
        .collect()
}

/// A script holds ONE `bulkWrite` per destination collection (a side and a namespace), even when
/// several pairs write to it, so ops are grouped by destination.
fn collection_sections(resolved: &[Resolved]) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for (i, r) in resolved.iter().enumerate() {
        for side in [Side::Target, Side::Source] {
            let ops: Vec<ApplyOp> = r
                .ops
                .iter()
                .filter(|op| destination_side(op) == side)
                .cloned()
                .collect();
            if ops.is_empty() {
                continue;
            }
            let ns = match side {
                Side::Target => r.spec.target.clone(),
                Side::Source => r.spec.source.clone(),
            };
            push_unit(
                &mut out,
                Head::Collection(side, ns),
                Unit { resolved: i, ops },
            );
        }
    }
    out
}

fn push_unit(out: &mut Vec<Section>, head: Head, unit: Unit) {
    match out.iter_mut().find(|s| s.head == head) {
        Some(section) => section.units.push(unit),
        None => out.push(Section {
            head,
            units: vec![unit],
        }),
    }
}

/// What the written items add up to; the source of the header counts and of the outcome.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Tally {
    pub counts: ExportCounts,
    pub failed: u64,
}

impl Tally {
    /// This tally after `item`.
    pub(super) fn with(mut self, item: &ExportItem) -> Self {
        match item {
            ExportItem::Write { op, .. } => match op.kind {
                ApplyKind::Insert => self.counts.insert += 1,
                ApplyKind::Replace => self.counts.replace += 1,
                ApplyKind::Delete => self.counts.delete += 1,
                ApplyKind::Fields => self.counts.update_fields += 1,
            },
            ExportItem::Skipped { .. } => self.counts.skipped += 1,
            ExportItem::Failed { .. } => self.failed += 1,
        }
        self
    }

    pub(super) fn written(&self) -> u64 {
        let c = self.counts;
        c.insert + c.replace + c.delete + c.update_fields
    }
}

/// A render or write error as a typed, value-free error.
pub(super) fn render_error(error: RenderError) -> CoreError {
    match error {
        RenderError::Io(_) => CoreError::ExportWriteFailed,
        RenderError::Unrenderable | RenderError::BadName | RenderError::Sequence => {
            CoreError::Unsupported("the changes cannot be exported".into())
        }
    }
}

type Out = BufWriter<File>;

/// The writer of the chosen format over the export file.
pub(super) enum FormatWriter {
    Script(ScriptWriter<Out>),
    Plan(PlanWriter<Out>),
}

impl FormatWriter {
    pub(super) fn new(format: ExportFormat, file: File) -> Self {
        let out = BufWriter::with_capacity(BUFFER, file);
        match format {
            ExportFormat::Mongosh => Self::Script(ScriptWriter::new(out)),
            ExportFormat::JsonPlan => Self::Plan(PlanWriter::new(out)),
        }
    }

    pub(super) fn begin(&mut self, header: &ExportHeader) -> Result<(), RenderError> {
        match self {
            Self::Script(w) => w.begin(header),
            Self::Plan(w) => w.begin(header),
        }
    }

    pub(super) fn open(&mut self, head: &Head) -> Result<(), RenderError> {
        match (self, head) {
            (Self::Script(w), Head::Collection(_, ns)) => w.collection(&ns.db, &ns.collection),
            (Self::Plan(w), Head::Pair(pair)) => w.pair(pair),
            _ => Err(RenderError::Sequence),
        }
    }

    pub(super) fn close(&mut self) -> Result<(), RenderError> {
        match self {
            Self::Script(w) => w.end_collection(),
            Self::Plan(_) => Ok(()),
        }
    }

    /// Writes the item. A failed item is not listed (the outcome counts it).
    pub(super) fn item(&mut self, item: ExportItem) -> Result<(), RenderError> {
        match item {
            ExportItem::Write {
                op,
                write,
                source_fp,
                target_fp,
            } => self.write(meta(&op, source_fp, target_fp), ShellOp::from(write)),
            ExportItem::Skipped {
                op,
                reason,
                source_fp,
                target_fp,
            } => self.skip(meta(&op, source_fp, target_fp), op.kind, reason),
            ExportItem::Failed { .. } => Ok(()),
        }
    }

    fn write(&mut self, meta: OpMeta, op: ShellOp) -> Result<(), RenderError> {
        match self {
            Self::Script(w) => w.op(&meta, &op),
            Self::Plan(w) => w.operation(&PlanOp::Write { meta, op }),
        }
    }

    fn skip(
        &mut self,
        meta: OpMeta,
        kind: ApplyKind,
        reason: crate::compare::export::ExportSkip,
    ) -> Result<(), RenderError> {
        match self {
            Self::Script(w) => w.skipped(&meta, kind, reason),
            Self::Plan(w) => w.operation(&PlanOp::Skipped { meta, kind, reason }),
        }
    }

    /// Finishes the document, flushes and hands the file back.
    pub(super) fn finish(self) -> Result<File, RenderError> {
        let out = match self {
            Self::Script(w) => w.finish()?,
            Self::Plan(w) => w.finish()?,
        };
        out.into_inner()
            .map_err(|e| RenderError::Io(e.error().kind()))
    }
}

fn meta(
    op: &ApplyOp,
    source: Option<crate::compare::fingerprint::Fingerprint>,
    target: Option<crate::compare::fingerprint::Fingerprint>,
) -> OpMeta {
    OpMeta {
        direction: op.direction,
        key: op.key.clone(),
        source,
        target,
    }
}

/// The header bytes the writer of `format` writes first, for `header` (exactly the prefix of the
/// file, so its length is known before the true counts are).
pub(super) fn header_bytes(
    format: ExportFormat,
    header: &ExportHeader,
) -> Result<Vec<u8>, RenderError> {
    let mut bytes = Vec::new();
    match format {
        ExportFormat::Mongosh => ScriptWriter::new(&mut bytes).begin(header)?,
        ExportFormat::JsonPlan => PlanWriter::new(&mut bytes).begin(header)?,
    }
    Ok(bytes)
}
