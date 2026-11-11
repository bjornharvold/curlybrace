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
    /// Masking the connection password changed the payload and broke it (a password equal to a
    /// JSON token such as `true` or `k`): the value is hidden. Holds nothing.
    Withheld,
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
            Self::Withheld => f.write_str("Withheld"),
        }
    }
}

type Json = serde_json::Value;
type Envelope = serde_json::Map<String, Json>;

/// Parse one payload line as the runner delivers it: already masked, with `masked` telling
/// whether masking changed it. The raw text is masked before parsing (nothing can leak), so a
/// password equal to a structural token breaks the JSON: that is [`ShellResult::Withheld`], not
/// "no value". A masked payload that still parses is shown, masked.
pub fn parse_scrubbed(payload: &str, masked: bool) -> Option<ShellResult> {
    match (parse_result(payload), masked) {
        (None, true) => Some(ShellResult::Withheld),
        (parsed, _) => parsed,
    }
}

/// Parse one payload line. `None` for anything that is not exactly a well-formed payload.
pub fn parse_result(payload: &str) -> Option<ShellResult> {
    // serde_json refuses nesting past 128 levels with an error (no stack overflow).
    let value: Json = serde_json::from_str(payload).ok()?;
    if depth_exceeds(&value, MAX_JSON_DEPTH) {
        return None;
    }
    let Json::Object(mut envelope) = value else {
        return None;
    };
    let kind = envelope.get("k")?.as_str()?.to_owned();
    match kind.as_str() {
        "cursor" => cursor(&mut envelope),
        "array" => array(&mut envelope),
        "object" => object(&mut envelope),
        "scalar" => scalar(&mut envelope),
        "too_big" => Some(ShellResult::TooBig {
            chars: whole_number(envelope.get("bytes")?)?,
        }),
        "unsupported" => Some(ShellResult::Unsupported {
            type_name: optional_string(envelope.get("t")?)?,
        }),
        _ => None,
    }
}

fn cursor(envelope: &mut Envelope) -> Option<ShellResult> {
    envelope.get("t")?.as_str()?;
    let more = envelope.get("more")?.as_bool()?;
    let (items, cut) = elements(envelope.remove("v")?)?;
    let docs = items
        .into_iter()
        .map(|item| match to_bson(item)? {
            Bson::Document(doc) => Some(doc),
            _ => None,
        })
        .collect::<Option<Vec<Document>>>()?;
    Some(ShellResult::Documents {
        kind: DocKind::Cursor,
        docs,
        more: more || cut,
        total: None,
    })
}

/// Each element is converted once, taking it; the array is documents only when every element
/// became one.
fn array(envelope: &mut Envelope) -> Option<ShellResult> {
    let n = whole_number(envelope.get("n")?)?;
    let more = envelope.get("more")?.as_bool()?;
    let (items, cut) = elements(envelope.remove("v")?)?;
    let kept = items.len() as u64;
    let more = more || cut || n > kept;
    let total = n.max(kept);
    let values = items
        .into_iter()
        .map(to_bson)
        .collect::<Option<Vec<Bson>>>()?;
    if !values.iter().all(|v| matches!(v, Bson::Document(_))) {
        return Some(ShellResult::Values {
            values,
            more,
            total,
        });
    }
    let docs = values
        .into_iter()
        .filter_map(|v| match v {
            Bson::Document(doc) => Some(doc),
            _ => None,
        })
        .collect();
    Some(ShellResult::Documents {
        kind: DocKind::Array,
        docs,
        more,
        total: Some(total),
    })
}

/// The emitter sends every non-array object as `object`, so a last value that is one BSON-typed
/// value (`new Date(0)`, `ObjectId(..)`, `NumberLong(5)`, `UUID()`, ...) comes as a JSON object
/// that converts to that value, not to a document: it is a scalar then (its `t` is not needed).
fn object(envelope: &mut Envelope) -> Option<ShellResult> {
    let type_name = optional_string(envelope.get("t")?)?;
    let v = envelope.remove("v")?;
    v.as_object()?;
    match to_bson(v)? {
        Bson::Document(doc) => Some(ShellResult::Object { type_name, doc }),
        value => Some(ShellResult::Scalar {
            value,
            undefined: false,
        }),
    }
}

fn scalar(envelope: &mut Envelope) -> Option<ShellResult> {
    let undefined = envelope.get("u")?.as_bool()?;
    Some(ShellResult::Scalar {
        value: to_bson(envelope.remove("v")?)?,
        undefined,
    })
}

/// The first `MAX_RESULT_ELEMENTS` of a JSON array, and whether any were cut.
fn elements(v: Json) -> Option<(Vec<Json>, bool)> {
    let Json::Array(mut all) = v else {
        return None;
    };
    let cut = all.len() > MAX_RESULT_ELEMENTS;
    all.truncate(MAX_RESULT_ELEMENTS);
    Some((all, cut))
}

/// Takes the value: `Bson::try_from` consumes it, so no subtree is cloned.
fn to_bson(v: Json) -> Option<Bson> {
    Bson::try_from(v).ok()
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
