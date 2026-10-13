//! mongosh rendering: every BSON value as an expression the shell evaluates back to the SAME
//! BSON type, and the script writer that puts them into `bulkWrite` statements.
//!
//! Value forms (checked against mongosh 2.13, whose JavaScript numbers are Int32 when integral):
//! * `Double`: a finite fractional number is a bare literal (Rust's shortest round-trip form);
//!   an integral one, `-0` included, is `Double(1.0)`, because a bare `1` would be stored as an
//!   Int32; `NaN`, `Infinity`, `-Infinity` are the JavaScript globals;
//! * `Int32` `NumberInt(n)`, `Int64` `NumberLong("n")`, `Decimal128` `NumberDecimal("s")`;
//! * `DateTime`: `ISODate("...")` for years 0000-9999 (also before 1970), otherwise
//!   `new Date(<ms>)` as far as a JavaScript Date reaches (+-8.64e15 ms), beyond that the value
//!   is refused;
//! * `Timestamp`: `Timestamp({ t: T, i: I })` (the object form is unambiguous about the order);
//! * `Binary`: `UUID("...")` for subtype 4 with 16 bytes, otherwise `BinData(subtype, "base64")`;
//! * regular expressions: `BSONRegExp(pattern, flags)` (a JavaScript `RegExp` rejects BSON-only
//!   flags such as `x`, and `/.../` literals need slash escaping);
//! * `Code("...")`, with scope `Code("...", {..})`; `Symbol` is `BSONSymbol("...")`;
//! * `MinKey()`, `MaxKey()`, `null`, booleans, strings as [`js_string`] literals;
//! * a regular expression with flags outside `imxlsu` is REFUSED (`BSONRegExp` throws on them);
//! * `Undefined` and `DbPointer` are REFUSED: the shell has no constructor for them (a bare
//!   `undefined` is written as `null` by the driver, a silently different value).
//!
//! Document keys are always string literals; `__proto__` is written as a computed key so it
//! becomes an own field instead of changing the prototype.

use std::io::Write;

use bson::spec::BinarySubtype;
use bson::{Binary, Bson, Document};

use super::sanitize::{comment_text, js_string};
use super::{
    ExportHeader, ExportSkip, OpMeta, RenderError, direction_name, fingerprint_hex, id_policy_name,
    kind_name, skip_name,
};
use crate::compare::apply::ApplyKind;
use crate::compare::diff::PathSegment;
use crate::store::WriteOp;

/// Deepest nesting rendered; BSON itself stops at 100 levels.
const MAX_DEPTH: usize = 128;
/// First and last millisecond whose year is 0000..=9999.
const ISO_MIN_MS: i64 = -62_167_219_200_000;
const ISO_MAX_MS: i64 = 253_402_300_799_999;
/// The range of a JavaScript `Date`.
const JS_DATE_MAX_MS: i64 = 8_640_000_000_000_000;

/// `v` as a mongosh expression.
pub fn render_value(v: &Bson) -> Result<String, RenderError> {
    let mut out = String::new();
    write_value(&mut out, v, 0)?;
    Ok(out)
}

/// `d` as a mongosh object literal.
pub fn render_document(d: &Document) -> Result<String, RenderError> {
    let mut out = String::new();
    write_document(&mut out, d, 0)?;
    Ok(out)
}

/// The dotted `$set`/`$unset` path of `segments`. Refuses exactly what the store refuses
/// (this IS the store's renderer): empty paths, a first segment that is not a field, `_id`,
/// `AnyElement`, and field names that are empty, contain `.` or NUL, or start with `$`.
pub fn render_path(segments: &[PathSegment]) -> Result<String, RenderError> {
    crate::mongo_write_fields::render_path(segments).map_err(|_| RenderError::BadName)
}

fn write_value(out: &mut String, v: &Bson, depth: usize) -> Result<(), RenderError> {
    if depth > MAX_DEPTH {
        return Err(RenderError::Unrenderable);
    }
    match v {
        Bson::Document(d) => write_document(out, d, depth),
        Bson::Array(items) => write_array(out, items, depth),
        Bson::JavaScriptCodeWithScope(c) => {
            out.push_str(&format!("Code({}, ", js_string(&c.code)));
            write_document(out, &c.scope, depth + 1)?;
            out.push(')');
            Ok(())
        }
        other => {
            out.push_str(&scalar(other)?);
            Ok(())
        }
    }
}

fn write_array(out: &mut String, items: &[Bson], depth: usize) -> Result<(), RenderError> {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_value(out, item, depth + 1)?;
    }
    out.push(']');
    Ok(())
}

fn write_document(out: &mut String, d: &Document, depth: usize) -> Result<(), RenderError> {
    if depth > MAX_DEPTH {
        return Err(RenderError::Unrenderable);
    }
    out.push('{');
    for (i, (key, value)) in d.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&key_literal(key));
        out.push_str(": ");
        write_value(out, value, depth + 1)?;
    }
    out.push('}');
    Ok(())
}

/// An object key: always a string literal; `__proto__` computed so it is an own property.
fn key_literal(key: &str) -> String {
    if key == "__proto__" {
        format!("[{}]", js_string(key))
    } else {
        js_string(key)
    }
}

/// The flags `BSONRegExp` accepts (it throws on others, `g` included).
fn valid_regex_flags(flags: &str) -> bool {
    flags.chars().all(|c| "imxlsu".contains(c))
}

/// Every non-container variant. Exhaustive: a new BSON type is a compile error here.
fn scalar(v: &Bson) -> Result<String, RenderError> {
    Ok(match v {
        Bson::Double(f) => double(*f),
        Bson::Int32(i) => format!("NumberInt({i})"),
        Bson::Int64(i) => format!("NumberLong(\"{i}\")"),
        Bson::Decimal128(d) => format!("NumberDecimal({})", js_string(&d.to_string())),
        Bson::String(s) => js_string(s),
        Bson::Boolean(b) => b.to_string(),
        Bson::Null => "null".to_string(),
        Bson::Symbol(s) => format!("BSONSymbol({})", js_string(s)),
        Bson::ObjectId(id) => format!("ObjectId(\"{}\")", id.to_hex()),
        Bson::DateTime(dt) => date(dt.timestamp_millis())?,
        Bson::Timestamp(ts) => format!("Timestamp({{ t: {}, i: {} }})", ts.time, ts.increment),
        Bson::Binary(bin) => binary(bin),
        Bson::RegularExpression(re) if !valid_regex_flags(&re.options) => {
            return Err(RenderError::Unrenderable);
        }
        Bson::RegularExpression(re) => format!(
            "BSONRegExp({}, {})",
            js_string(&re.pattern),
            js_string(&re.options)
        ),
        Bson::JavaScriptCode(code) => format!("Code({})", js_string(code)),
        Bson::MinKey => "MinKey()".to_string(),
        Bson::MaxKey => "MaxKey()".to_string(),
        Bson::Undefined
        | Bson::DbPointer(_)
        | Bson::Document(_)
        | Bson::Array(_)
        | Bson::JavaScriptCodeWithScope(_) => return Err(RenderError::Unrenderable),
    })
}

fn double(f: f64) -> String {
    if f.is_nan() {
        "NaN".to_string()
    } else if f.is_infinite() {
        (if f > 0.0 { "Infinity" } else { "-Infinity" }).to_string()
    } else if f.fract() == 0.0 {
        format!("Double({f:?})")
    } else {
        format!("{f:?}")
    }
}

fn date(ms: i64) -> Result<String, RenderError> {
    if (ISO_MIN_MS..=ISO_MAX_MS).contains(&ms) {
        Ok(format!("ISODate(\"{}\")", iso_string(ms)))
    } else if ms.unsigned_abs() <= JS_DATE_MAX_MS.unsigned_abs() {
        Ok(format!("new Date({ms})"))
    } else {
        Err(RenderError::Unrenderable)
    }
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for a millisecond count within years 0000..=9999.
fn iso_string(ms: i64) -> String {
    const DAY_MS: i64 = 86_400_000;
    let days = ms.div_euclid(DAY_MS);
    let rest = ms.rem_euclid(DAY_MS);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1_000 % 60,
        rest % 1_000
    )
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn binary(bin: &Binary) -> String {
    if bin.subtype == BinarySubtype::Uuid && bin.bytes.len() == 16 {
        return format!("UUID(\"{}\")", uuid_text(&bin.bytes));
    }
    format!(
        "BinData({}, \"{}\")",
        u8::from(bin.subtype),
        base64(&bin.bytes)
    )
}

fn uuid_text(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Standard base64 with padding (RFC 4648).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// One write of the script. Mirrors [`WriteOp`] without tying the renderer to the store.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellOp {
    Insert {
        doc: Document,
    },
    Replace {
        id: Bson,
        doc: Document,
    },
    Delete {
        id: Bson,
    },
    UpdateFields {
        id: Bson,
        set: Vec<(Vec<PathSegment>, Bson)>,
        unset: Vec<Vec<PathSegment>>,
    },
}

impl From<WriteOp> for ShellOp {
    fn from(op: WriteOp) -> Self {
        match op {
            WriteOp::Insert { doc } => Self::Insert { doc },
            WriteOp::Replace { id, doc } => Self::Replace { id, doc },
            WriteOp::Delete { id } => Self::Delete { id },
            WriteOp::UpdateFields { id, set, unset } => Self::UpdateFields { id, set, unset },
        }
    }
}

impl ShellOp {
    /// The kind name used in comments and plans.
    pub(crate) fn kind(&self) -> &'static str {
        kind_name(match self {
            Self::Insert { .. } => ApplyKind::Insert,
            Self::Replace { .. } => ApplyKind::Replace,
            Self::Delete { .. } => ApplyKind::Delete,
            Self::UpdateFields { .. } => ApplyKind::Fields,
        })
    }
}

/// `{"_id": <id>}`
fn id_filter(id: &Bson) -> Result<String, RenderError> {
    Ok(format!("{{\"_id\": {}}}", render_value(id)?))
}

/// The `updateOne` filter and update of a field update: EXACTLY the store's statement
/// (`mongo_write_fields::build_statement`), guards included (array / object shape of the
/// parents, existence of an indexed element), so the script refuses and guards what the apply
/// does. Path-name refusals are `BadName`; everything else the store refuses (nothing to do,
/// unset through an array, conflicting paths) is `Unrenderable`.
fn update_parts(
    id: &Bson,
    set: &[(Vec<PathSegment>, Bson)],
    unset: &[Vec<PathSegment>],
) -> Result<(String, String), RenderError> {
    for path in set
        .iter()
        .map(|(p, _)| p.as_slice())
        .chain(unset.iter().map(Vec::as_slice))
    {
        render_path(path)?;
    }
    let statement = crate::mongo_write_fields::build_statement(id, set, unset)
        .map_err(|_| RenderError::Unrenderable)?;
    let update = statement
        .statement
        .get_document("u")
        .map_err(|_| RenderError::Unrenderable)?;
    Ok((render_document(&statement.query)?, render_document(update)?))
}

/// Whether `op` can be rendered at all; the JSON plan uses this so that it refuses exactly the
/// operations the script refuses.
pub(crate) fn check_op(op: &ShellOp) -> Result<(), RenderError> {
    op_item(op).map(|_| ())
}

/// The `bulkWrite` item for `op`.
fn op_item(op: &ShellOp) -> Result<String, RenderError> {
    Ok(match op {
        ShellOp::Insert { doc } => {
            format!("{{ insertOne: {{ document: {} }} }}", render_document(doc)?)
        }
        ShellOp::Replace { id, doc } => format!(
            "{{ replaceOne: {{ filter: {}, replacement: {} }} }}",
            id_filter(id)?,
            render_document(doc)?
        ),
        ShellOp::Delete { id } => {
            format!("{{ deleteOne: {{ filter: {} }} }}", id_filter(id)?)
        }
        ShellOp::UpdateFields { id, set, unset } => {
            let (filter, update) = update_parts(id, set, unset)?;
            format!("{{ updateOne: {{ filter: {filter}, update: {update} }} }}")
        }
    })
}

/// `direction kind source=<hex|none> target=<hex|none> key=[..]`, as one safe comment tail.
fn describe(meta: &OpMeta, kind: &str) -> String {
    let fp = |f: &Option<crate::compare::fingerprint::Fingerprint>| {
        f.as_ref()
            .map_or_else(|| "none".to_string(), fingerprint_hex)
    };
    let key = meta
        .key
        .0
        .iter()
        .map(render_value)
        .collect::<Result<Vec<_>, _>>()
        .map_or_else(
            |_| "<unrenderable>".to_string(),
            |v| format!("[{}]", v.join(", ")),
        );
    comment_text(&format!(
        "{} {kind} source={} target={} key={key}",
        direction_name(meta.direction),
        fp(&meta.source),
        fp(&meta.target)
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    New,
    Idle,
    Collection { opened: bool },
}

/// Writes a mongosh script (note: with `ordered: false` a failed item, e.g. a duplicate key,
/// still makes `bulkWrite` throw, and the shell then stops: later statements do not run; the
/// header says so): a comment header, then per target collection ONE
/// `db.getSiblingDB(..).getCollection(..).bulkWrite([..], { ordered: false });` statement
/// (none when the collection has no writes). Each operation is rendered completely before any
/// of it is written, so a refused operation leaves the output as it was.
pub struct ScriptWriter<W: Write> {
    inner: W,
    state: State,
    statement_open: String,
}

impl<W: Write> ScriptWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            state: State::New,
            statement_open: String::new(),
        }
    }

    /// The comment header. Must be the first call.
    pub fn begin(&mut self, header: &ExportHeader) -> Result<(), RenderError> {
        if self.state != State::New {
            return Err(RenderError::Sequence);
        }
        let c = header.counts;
        let label_line = format!("// label: {}", comment_text(&header.label));
        let text = format!(
            "// curlybrace data compare: mongosh script\n\
             {}\n\
             // operations: insert {}, replace {}, delete {}, updateFields {}, skipped {}\n\
             // id policy: {}\n\
             //\n\
             // Review this script before you run it: it writes to the databases it names.\n\
             // It was written from an earlier comparison, and documents may have changed since.\n\
             // Skipped operations are listed as comments and are never written.\n\
             // bulkWrite runs every item of a statement even with ordered: false, but if an item fails (a\n\
             // duplicate key, say) it throws, and mongosh then stops: the later statements of this script do not run.\n",
            label_line.trim_end(),
            c.insert,
            c.replace,
            c.delete,
            c.update_fields,
            c.skipped,
            id_policy_name(header.id_policy),
        );
        self.inner.write_all(text.as_bytes())?;
        self.state = State::Idle;
        Ok(())
    }

    /// Starts the section of one target collection.
    pub fn collection(&mut self, db: &str, name: &str) -> Result<(), RenderError> {
        if self.state != State::Idle {
            return Err(RenderError::Sequence);
        }
        if [db, name].iter().any(|n| n.is_empty() || n.contains('\0')) {
            return Err(RenderError::BadName);
        }
        let text = format!(
            "\n// collection: {}\n",
            comment_text(&format!("{db}.{name}"))
        );
        self.inner.write_all(text.as_bytes())?;
        self.statement_open = format!(
            "db.getSiblingDB({}).getCollection({}).bulkWrite([\n",
            js_string(db),
            js_string(name)
        );
        self.state = State::Collection { opened: false };
        Ok(())
    }

    /// One write, preceded by a comment with its fingerprints and key.
    pub fn op(&mut self, meta: &OpMeta, op: &ShellOp) -> Result<(), RenderError> {
        let State::Collection { opened } = self.state else {
            return Err(RenderError::Sequence);
        };
        let body = format!("  // {}\n  {},\n", describe(meta, op.kind()), op_item(op)?);
        if !opened {
            self.inner.write_all(self.statement_open.as_bytes())?;
            self.state = State::Collection { opened: true };
        }
        self.inner.write_all(body.as_bytes())?;
        Ok(())
    }

    /// An operation that is listed but not written.
    pub fn skipped(
        &mut self,
        meta: &OpMeta,
        kind: ApplyKind,
        reason: impl Into<ExportSkip>,
    ) -> Result<(), RenderError> {
        if !matches!(self.state, State::Collection { .. }) {
            return Err(RenderError::Sequence);
        }
        let line = format!(
            "  // skipped ({}) {}\n",
            skip_name(reason.into()),
            describe(meta, kind_name(kind))
        );
        self.inner.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Ends the section, closing the statement when it has one.
    pub fn end_collection(&mut self) -> Result<(), RenderError> {
        let State::Collection { opened } = self.state else {
            return Err(RenderError::Sequence);
        };
        if opened {
            self.inner.write_all(b"], { ordered: false });\n")?;
        }
        self.state = State::Idle;
        Ok(())
    }

    /// Flushes and returns the sink. Refused inside a collection: a statement must be closed.
    pub fn finish(mut self) -> Result<W, RenderError> {
        if self.state != State::Idle {
            return Err(RenderError::Sequence);
        }
        self.inner.flush()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
#[path = "mongosh_script_tests.rs"]
mod script_tests;
#[cfg(test)]
#[path = "mongosh_tests.rs"]
mod value_tests;
