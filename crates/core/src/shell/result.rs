//! Parsing of the structured result payload: the one canonical-EJSON line the epilogue emits for
//! the last value of a run, turned into typed data the Table and Tree views can show.
use bson::{Bson, Document};

/// The most elements of one result that are kept (matches the emitter's cap).
pub const MAX_RESULT_ELEMENTS: usize = 200;

/// A payload that nests deeper than this is refused.
const MAX_JSON_DEPTH: usize = 100;

/// Where a list of documents came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Cursor,
    Array,
}

/// The last value of a run, as far as it could be lifted out.
#[derive(Clone, PartialEq)]
pub enum ShellResult {
    Documents {
        kind: DocKind,
        docs: Vec<Document>,
        more: bool,
        total: Option<u64>,
    },
    Object {
        type_name: Option<String>,
        doc: Document,
    },
    Scalar {
        value: Bson,
        undefined: bool,
    },
    Values {
        values: Vec<Bson>,
        more: bool,
        total: u64,
    },
    /// The value's EJSON was over the emitter's cap. `chars` is its length as the emitter
    /// counts it: JavaScript string length, UTF-16 units (about characters, not bytes). On the
    /// wire the field is still called `bytes`.
    TooBig {
        chars: u64,
    },
    Unsupported {
        type_name: Option<String>,
    },
}

// Hand-written: documents and values may hold secrets, so only shapes, counts and flags are shown.
impl std::fmt::Debug for ShellResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Documents {
                kind,
                docs,
                more,
                total,
            } => f
                .debug_struct("Documents")
                .field("kind", kind)
                .field("docs", &docs.len())
                .field("more", more)
                .field("total", total)
                .finish(),
            Self::Object { type_name, doc } => f
                .debug_struct("Object")
                .field("type_name", type_name)
                .field("fields", &doc.len())
                .finish(),
            Self::Scalar { undefined, .. } => f
                .debug_struct("Scalar")
                .field("undefined", undefined)
                .finish_non_exhaustive(),
            Self::Values {
                values,
                more,
                total,
            } => f
                .debug_struct("Values")
                .field("values", &values.len())
                .field("more", more)
                .field("total", total)
                .finish(),
            Self::TooBig { chars } => f.debug_struct("TooBig").field("chars", chars).finish(),
            Self::Unsupported { type_name } => f
                .debug_struct("Unsupported")
                .field("type_name", type_name)
                .finish(),
        }
    }
}

type Json = serde_json::Value;
type Envelope = serde_json::Map<String, Json>;

/// Parse one payload line. `None` for anything that is not exactly a well-formed payload.
pub fn parse_result(payload: &str) -> Option<ShellResult> {
    // serde_json refuses nesting past 128 levels with an error (no stack overflow).
    let value: Json = serde_json::from_str(payload).ok()?;
    if depth_exceeds(&value, MAX_JSON_DEPTH) {
        return None;
    }
    let envelope = value.as_object()?;
    match envelope.get("k")?.as_str()? {
        "cursor" => cursor(envelope),
        "array" => array(envelope),
        "object" => object(envelope),
        "scalar" => scalar(envelope),
        "too_big" => Some(ShellResult::TooBig {
            chars: whole_number(envelope.get("bytes")?)?,
        }),
        "unsupported" => Some(ShellResult::Unsupported {
            type_name: optional_string(envelope.get("t")?)?,
        }),
        _ => None,
    }
}

fn cursor(envelope: &Envelope) -> Option<ShellResult> {
    envelope.get("t")?.as_str()?;
    let (items, cut) = elements(envelope.get("v")?)?;
    let docs = items
        .iter()
        .map(to_document)
        .collect::<Option<Vec<Document>>>()?;
    Some(ShellResult::Documents {
        kind: DocKind::Cursor,
        docs,
        more: envelope.get("more")?.as_bool()? || cut,
        total: None,
    })
}

fn array(envelope: &Envelope) -> Option<ShellResult> {
    let n = whole_number(envelope.get("n")?)?;
    let (items, cut) = elements(envelope.get("v")?)?;
    let kept = items.len() as u64;
    let more = envelope.get("more")?.as_bool()? || cut || n > kept;
    let total = n.max(kept);
    if let Some(docs) = items
        .iter()
        .map(to_document)
        .collect::<Option<Vec<Document>>>()
    {
        return Some(ShellResult::Documents {
            kind: DocKind::Array,
            docs,
            more,
            total: Some(total),
        });
    }
    let values = items.iter().map(to_bson).collect::<Option<Vec<Bson>>>()?;
    Some(ShellResult::Values {
        values,
        more,
        total,
    })
}

fn object(envelope: &Envelope) -> Option<ShellResult> {
    Some(ShellResult::Object {
        type_name: optional_string(envelope.get("t")?)?,
        doc: to_document(envelope.get("v")?)?,
    })
}

fn scalar(envelope: &Envelope) -> Option<ShellResult> {
    Some(ShellResult::Scalar {
        value: to_bson(envelope.get("v")?)?,
        undefined: envelope.get("u")?.as_bool()?,
    })
}

/// The first `MAX_RESULT_ELEMENTS` of a JSON array, and whether any were cut.
fn elements(v: &Json) -> Option<(&[Json], bool)> {
    let all = v.as_array()?;
    let kept = all.len().min(MAX_RESULT_ELEMENTS);
    Some((&all[..kept], all.len() > kept))
}

fn to_bson(v: &Json) -> Option<Bson> {
    Bson::try_from(v.clone()).ok()
}

fn to_document(v: &Json) -> Option<Document> {
    match to_bson(v)? {
        Bson::Document(doc) => Some(doc),
        _ => None,
    }
}

/// `Some(None)` for JSON null, `Some(Some(s))` for a string, `None` for anything else.
fn optional_string(v: &Json) -> Option<Option<String>> {
    match v {
        Json::Null => Some(None),
        Json::String(s) => Some(Some(s.clone())),
        _ => None,
    }
}

/// A non-negative whole number, plain or wrapped as `$numberInt` / `$numberLong` / `$numberDouble`.
fn whole_number(v: &Json) -> Option<u64> {
    match v {
        Json::Number(n) => n.as_u64(),
        Json::Object(o) if o.len() == 1 => {
            let (key, inner) = o.iter().next()?;
            let text = inner.as_str()?;
            match key.as_str() {
                "$numberInt" | "$numberLong" => text.parse().ok(),
                "$numberDouble" => {
                    let f: f64 = text.parse().ok()?;
                    (f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f < u64::MAX as f64)
                        .then_some(f as u64)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// True when `v` nests deeper than `limit` levels (iterative, so no stack growth).
fn depth_exceeds(v: &Json, limit: usize) -> bool {
    let mut stack = vec![(v, 1usize)];
    while let Some((node, depth)) = stack.pop() {
        let children: Vec<&Json> = match node {
            Json::Array(a) => a.iter().collect(),
            Json::Object(o) => o.values().collect(),
            _ => continue,
        };
        if depth > limit {
            return true;
        }
        stack.extend(children.into_iter().map(|c| (c, depth + 1)));
    }
    false
}

#[cfg(test)]
#[path = "result_tests.rs"]
mod tests;
