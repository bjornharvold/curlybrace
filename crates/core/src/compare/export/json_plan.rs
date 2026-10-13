//! The JSON plan: the same operations as the script, as data. Streamed one operation at a time,
//! so memory stays bounded.
//!
//! ```text
//! { "header": { "format": "curlybrace-compare-plan", "version": 1, "label": .., "counts": {..},
//!               "idPolicy": "keep" | "generate" },
//!   "pairs": [ { "source": {db, collection}, "target": {db, collection}, "key": [fields],
//!                "operations": [ { "op": "insert" | "replace" | "delete" | "updateFields" |
//!                                  "skipped", "direction", "key", "sourceFingerprint",
//!                                  "targetFingerprint", .. } ] } ] }
//! ```
//!
//! Keys, documents and values are canonical extended JSON; field paths are arrays of segments
//! (a string for a field, a number for an array index), never dotted strings. Fingerprints are
//! hex or `null`. The plan refuses exactly the operations the script refuses (`RenderError::Unrenderable`/`BadName`).

use std::io::Write;

use bson::{Bson, Document};
use serde_json::{Map, Number, Value, json};

use super::mongosh::{ShellOp, check_op};
use super::{
    ExportHeader, OpMeta, RenderError, direction_name, fingerprint_hex, id_policy_name, kind_name,
    skip_name,
};
use crate::compare::apply::{ApplyKind, SkipReason};
use crate::compare::diff::PathSegment;
use crate::store::Namespace;

/// The `format` value of a plan header.
pub const PLAN_FORMAT: &str = "curlybrace-compare-plan";
/// The plan format version.
pub const PLAN_VERSION: u64 = 1;

/// One compared pair: where the plan reads from and writes to, and the key fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPair {
    pub source: Namespace,
    pub target: Namespace,
    pub key: Vec<String>,
}

/// One entry of a pair's `operations`.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanOp {
    /// A write an apply would do.
    Write { meta: OpMeta, op: ShellOp },
    /// An operation the export could not plan (`kind` is what was queued); never a write.
    Skipped {
        meta: OpMeta,
        kind: ApplyKind,
        reason: SkipReason,
    },
}

fn canonical(v: &Bson) -> Value {
    v.clone().into_canonical_extjson()
}

fn canonical_doc(d: &Document) -> Value {
    canonical(&Bson::Document(d.clone()))
}

fn path_value(path: &[PathSegment]) -> Result<Value, RenderError> {
    path.iter()
        .map(|segment| match segment {
            PathSegment::Field(name) => Ok(Value::String(name.clone())),
            PathSegment::Index(i) => Ok(Value::Number(Number::from(*i as u64))),
            PathSegment::AnyElement => Err(RenderError::Unrenderable),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn paths_value(paths: &[Vec<PathSegment>]) -> Result<Value, RenderError> {
    paths
        .iter()
        .map(|p| path_value(p))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn sets_value(set: &[(Vec<PathSegment>, Bson)]) -> Result<Value, RenderError> {
    set.iter()
        .map(|(path, value)| Ok(json!({ "path": path_value(path)?, "value": canonical(value) })))
        .collect::<Result<Vec<_>, RenderError>>()
        .map(Value::Array)
}

fn header_value(header: &ExportHeader) -> Value {
    let c = header.counts;
    json!({
        "format": PLAN_FORMAT,
        "version": PLAN_VERSION,
        "label": header.label,
        "counts": {
            "insert": c.insert,
            "replace": c.replace,
            "delete": c.delete,
            "updateFields": c.update_fields,
            "skipped": c.skipped,
        },
        "idPolicy": id_policy_name(header.id_policy),
    })
}

fn valid_namespace(ns: &Namespace) -> bool {
    [&ns.db, &ns.collection]
        .iter()
        .all(|n| !n.is_empty() && !n.contains('\0'))
}

fn namespace_value(ns: &Namespace) -> Value {
    json!({ "db": ns.db, "collection": ns.collection })
}

/// The members every operation has, in a fixed order.
fn common(op: &str, meta: &OpMeta) -> Map<String, Value> {
    let fp = |f: &Option<crate::compare::fingerprint::Fingerprint>| {
        f.as_ref()
            .map_or(Value::Null, |f| Value::String(fingerprint_hex(f)))
    };
    let mut m = Map::new();
    m.insert("op".into(), Value::String(op.to_string()));
    m.insert(
        "direction".into(),
        Value::String(direction_name(meta.direction).to_string()),
    );
    m.insert(
        "key".into(),
        Value::Array(meta.key.0.iter().map(canonical).collect()),
    );
    m.insert("sourceFingerprint".into(), fp(&meta.source));
    m.insert("targetFingerprint".into(), fp(&meta.target));
    m
}

fn write_value(meta: &OpMeta, op: &ShellOp) -> Result<Value, RenderError> {
    let mut m = common(op.kind(), meta);
    match op {
        ShellOp::Insert { doc } => {
            m.insert("document".into(), canonical_doc(doc));
        }
        ShellOp::Replace { id, doc } => {
            m.insert("id".into(), canonical(id));
            m.insert("document".into(), canonical_doc(doc));
        }
        ShellOp::Delete { id } => {
            m.insert("id".into(), canonical(id));
        }
        ShellOp::UpdateFields { id, set, unset } => {
            m.insert("id".into(), canonical(id));
            m.insert("set".into(), sets_value(set)?);
            m.insert("unset".into(), paths_value(unset)?);
        }
    }
    Ok(Value::Object(m))
}

fn skipped_value(meta: &OpMeta, kind: ApplyKind, reason: SkipReason) -> Value {
    let mut m = common("skipped", meta);
    m.insert("kind".into(), Value::String(kind_name(kind).to_string()));
    m.insert(
        "reason".into(),
        Value::String(skip_name(reason).to_string()),
    );
    Value::Object(m)
}

/// Compact JSON; U+2028/2029 are escaped too, so the text is safe in any JavaScript context.
fn to_text(v: &Value) -> Result<String, RenderError> {
    serde_json::to_string(v)
        .map(|s| {
            s.replace('\u{2028}', "\\u2028")
                .replace('\u{2029}', "\\u2029")
        })
        .map_err(|_| RenderError::Unrenderable)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    New,
    Pairs {
        any_pair: bool,
        in_pair: bool,
        ops: usize,
    },
}

/// Writes a plan: [`begin`](Self::begin), then per pair [`pair`](Self::pair) and its
/// [`operation`](Self::operation)s, then [`finish`](Self::finish).
pub struct PlanWriter<W: Write> {
    inner: W,
    state: State,
}

impl<W: Write> PlanWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            state: State::New,
        }
    }

    /// The header. Must be the first call.
    pub fn begin(&mut self, header: &ExportHeader) -> Result<(), RenderError> {
        if self.state != State::New {
            return Err(RenderError::Sequence);
        }
        let text = format!(
            "{{\"header\":{},\"pairs\":[",
            to_text(&header_value(header))?
        );
        self.inner.write_all(text.as_bytes())?;
        self.state = State::Pairs {
            any_pair: false,
            in_pair: false,
            ops: 0,
        };
        Ok(())
    }

    /// Starts the next pair (closing the previous one).
    pub fn pair(&mut self, pair: &PlanPair) -> Result<(), RenderError> {
        let State::Pairs {
            any_pair, in_pair, ..
        } = self.state
        else {
            return Err(RenderError::Sequence);
        };
        if !valid_namespace(&pair.source) || !valid_namespace(&pair.target) {
            return Err(RenderError::BadName);
        }
        let head = to_text(&json!({
            "source": namespace_value(&pair.source),
            "target": namespace_value(&pair.target),
            "key": pair.key,
        }))?;
        // `head` is `{..}`: reopen it to append the operations array.
        let open = format!("\n{},\"operations\":[", &head[..head.len() - 1]);
        let mut text = String::new();
        if in_pair {
            text.push_str("\n]}");
        }
        if any_pair {
            text.push(',');
        }
        text.push_str(&open);
        self.inner.write_all(text.as_bytes())?;
        self.state = State::Pairs {
            any_pair: true,
            in_pair: true,
            ops: 0,
        };
        Ok(())
    }

    /// One operation of the current pair.
    pub fn operation(&mut self, op: &PlanOp) -> Result<(), RenderError> {
        let State::Pairs {
            any_pair,
            in_pair: true,
            ops,
        } = self.state
        else {
            return Err(RenderError::Sequence);
        };
        let value = match op {
            PlanOp::Write { meta, op } => {
                // Refuse what the script refuses (unsupported values, unaddressable or
                // conflicting update paths), so the two exports never disagree.
                check_op(op)?;
                write_value(meta, op)?
            }
            PlanOp::Skipped { meta, kind, reason } => skipped_value(meta, *kind, *reason),
        };
        let text = format!("{}\n{}", if ops > 0 { "," } else { "" }, to_text(&value)?);
        self.inner.write_all(text.as_bytes())?;
        self.state = State::Pairs {
            any_pair,
            in_pair: true,
            ops: ops + 1,
        };
        Ok(())
    }

    /// Closes the document, flushes and returns the sink.
    pub fn finish(mut self) -> Result<W, RenderError> {
        let State::Pairs { in_pair, .. } = self.state else {
            return Err(RenderError::Sequence);
        };
        let tail = if in_pair { "\n]}\n]}\n" } else { "\n]}\n" };
        self.inner.write_all(tail.as_bytes())?;
        self.inner.flush()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
#[path = "json_plan_tests.rs"]
mod tests;
