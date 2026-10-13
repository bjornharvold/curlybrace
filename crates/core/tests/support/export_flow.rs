//! Helpers for the tier-1 export tests: gating, seeding, export commands and a small RFC 4180
//! reader (written here on purpose, so the tests do not trust the writer's own code).
#![allow(dead_code)]

use std::path::Path;

use bson::{Bson, Document, doc};
use curlybrace_core::compare::apply::{ApplyKind, ApplyOp, Direction};
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, ExportOutcome, ExportRequest, ReportRequest, ReportTarget,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::RuntimeHandle;
use curlybrace_core::store::Namespace;

use super::compare_flow::{SESSION, keys, op, send};
use super::next_compare;

/// The shared gate plus the rule that the server address must be chosen explicitly.
pub fn enabled(test: &str) -> bool {
    if !super::local_enabled(test) {
        return false;
    }
    let explicit = std::env::var(curlybrace_testkit::ENV_LOCAL_URI)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if !explicit {
        eprintln!(
            "SKIPPED {test}: set {} to your own cb-it-dev-* container",
            curlybrace_testkit::ENV_LOCAL_URI
        );
    }
    explicit
}

/// The gate for tests that also run the generated script with the container's `mongosh`.
pub fn enabled_with_mongosh(test: &str) -> Option<String> {
    if !enabled(test) {
        return None;
    }
    super::mongosh::mongosh_container(test)
}

/// Seeds `docs` into `ns`, creating the collection even when `docs` is empty.
pub async fn seed(client: &mongodb::Client, ns: &Namespace, docs: Vec<Document>) {
    let db = client.database(&ns.db);
    if docs.is_empty() {
        db.create_collection(&ns.collection).await.unwrap();
    } else {
        for chunk in docs.chunks(5_000) {
            db.collection::<Document>(&ns.collection)
                .insert_many(chunk.to_vec())
                .await
                .unwrap();
        }
    }
}

/// Insert/replace/delete ops (source copied onto target) for every row of a pair.
pub async fn all_ops(rt: &mut RuntimeHandle, pair: u32) -> Vec<ApplyOp> {
    let mut ops = Vec::new();
    for (category, kind) in [
        (Category::OnlyInSource, ApplyKind::Insert),
        (Category::Different, ApplyKind::Replace),
        (Category::OnlyInTarget, ApplyKind::Delete),
    ] {
        for k in keys(rt, pair, category).await {
            ops.push(op(pair, k, kind));
        }
    }
    ops
}

/// The same ops re-addressed to another pair.
pub fn for_pair(ops: &[ApplyOp], pair: u32) -> Vec<ApplyOp> {
    ops.iter().map(|o| ApplyOp { pair, ..o.clone() }).collect()
}

pub fn request(
    format: curlybrace_core::compare::protocol::ExportFormat,
    ops: Vec<ApplyOp>,
) -> ExportRequest {
    ExportRequest {
        format,
        label: "tier-1".to_string(),
        id_policy: curlybrace_core::compare::apply::IdPolicy::Keep,
        ops,
        bulk: Vec::new(),
    }
}

/// Sends `Export` and collects events until `ExportDone`; a `Failed` is returned as the error.
pub async fn export(
    rt: &mut RuntimeHandle,
    run: u64,
    request: ExportRequest,
    path: &Path,
) -> Result<ExportOutcome, (&'static str, CoreError)> {
    send(
        rt,
        CompareCommand::Export {
            session: SESSION,
            run,
            request,
            path: path.to_path_buf().into(),
        },
    );
    loop {
        match next_compare(rt).await {
            CompareEvent::ExportProgress { .. } => {}
            CompareEvent::ExportDone { outcome, .. } => return Ok(outcome),
            CompareEvent::Failed { context, error, .. } => return Err((context, error)),
            other => panic!("{other:?}"),
        }
    }
}

/// `export`, but a refusal fails the test.
pub async fn export_ok(
    rt: &mut RuntimeHandle,
    run: u64,
    request: ExportRequest,
    path: &Path,
) -> ExportOutcome {
    export(rt, run, request, path)
        .await
        .unwrap_or_else(|e| panic!("export refused: {e:?}"))
}

/// Sends `ExportReport`; returns the clipboard text (if any) and the outcome.
pub async fn report(
    rt: &mut RuntimeHandle,
    run: u64,
    request: ReportRequest,
    target: ReportTarget,
) -> Result<(Option<String>, ExportOutcome), (&'static str, CoreError)> {
    send(
        rt,
        CompareCommand::ExportReport {
            session: SESSION,
            run,
            request,
            target,
        },
    );
    let mut text = None;
    loop {
        match next_compare(rt).await {
            CompareEvent::ExportProgress { .. } => {}
            CompareEvent::ExportText { text: t, .. } => text = Some(t.into_string()),
            CompareEvent::ExportDone { outcome, .. } => return Ok((text, outcome)),
            CompareEvent::Failed { context, error, .. } => return Err((context, error)),
            other => panic!("{other:?}"),
        }
    }
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("the export file")
}

/// A field-update op for one document (source fields copied onto the target).
pub fn fields_op(
    pair: u32,
    key: Bson,
    paths: Vec<Vec<curlybrace_core::compare::diff::PathSegment>>,
) -> ApplyOp {
    ApplyOp {
        pair,
        key: super::compare_flow::key(key),
        kind: ApplyKind::Fields,
        direction: Direction::SourceToTarget,
        fields: paths,
    }
}

/// Raw BSON bytes of one field of a document (type tag, name and value).
pub fn field_bytes(d: &Document, name: &str) -> Option<Vec<u8>> {
    let value = d.get(name)?;
    Some(bson::to_vec(&doc! { name: value.clone() }).unwrap())
}

/// An RFC 4180 reader: rows of cells; quoted cells may hold commas, quotes (doubled) and line
/// breaks. Lines end with CRLF or LF.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut cell) = (Vec::new(), String::new());
    let (mut quoted, mut was_quoted) = (false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => quoted = false,
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() && !was_quoted => {
                quoted = true;
                was_quoted = true;
            }
            ',' => {
                row.push(std::mem::take(&mut cell));
                was_quoted = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                was_quoted = false;
            }
            _ => cell.push(c),
        }
    }
    assert!(!quoted, "unterminated quoted cell");
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}
