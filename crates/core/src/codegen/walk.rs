//! The one walk over a BSON value tree every renderer shares.
//!
//! A language supplies a [`Syntax`] (how each leaf type is spelled, how keys
//! are written, the indent unit); [`render`] does the layout. The `match` on
//! [`Bson`] has no wildcard arm, so a new BSON variant is a compile error here
//! rather than a silently dropped value.

use bson::spec::BinarySubtype;
use bson::{Binary, Bson, Document};

use crate::codegen::escape::indent;
use crate::codegen::model::CodegenError;

/// How one language spells BSON values. Leaves return the literal text.
pub(crate) trait Syntax {
    fn indent_unit(&self) -> &'static str;
    /// Space inside inline braces: `{ a: 1 }` (JS) or `{"a": 1}` (Python).
    fn brace_pad(&self) -> &'static str;
    /// A document key as it appears before the colon.
    fn key(&self, key: &str) -> String;

    fn double(&self, v: f64) -> String;
    fn string(&self, v: &str) -> String;
    fn int32(&self, v: i32) -> String;
    fn int64(&self, v: i64) -> String;
    fn boolean(&self, v: bool) -> String;
    fn null(&self) -> String;
    fn undefined(&self) -> String;
    fn object_id(&self, hex: &str) -> String;
    fn date_time(&self, millis: i64) -> Result<String, CodegenError>;
    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError>;
    /// `bytes` is the little-endian 128-bit encoding.
    fn decimal128(&self, text: &str, bytes: [u8; 16]) -> String;
    fn regex(&self, pattern: &str, options: &str) -> String;
    fn timestamp(&self, time: u32, increment: u32) -> String;
    fn min_key(&self) -> String;
    fn max_key(&self) -> String;
    fn code(&self, code: &str) -> String;
    /// `scope` is the already rendered scope document.
    fn code_with_scope(&self, code: &str, scope: &str) -> String;
    fn symbol(&self, text: &str) -> String;
    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String;

    /// A document from `(rendered key, rendered value)` pairs; never empty.
    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        let items: Vec<String> = entries.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        lay_out_with(
            self.indent_unit(),
            ("{", "}"),
            self.brace_pad(),
            &items,
            depth,
            true,
        )
    }
    /// An array of rendered items; never empty.
    fn array(&self, items: &[String], depth: usize) -> String {
        lay_out_with(self.indent_unit(), ("[", "]"), "", items, depth, true)
    }
    /// The list of rendered stage documents.
    fn pipeline(&self, items: &[String], depth: usize) -> String {
        self.array(items, depth)
    }
    fn empty_document(&self) -> String {
        "{}".into()
    }
    fn empty_array(&self) -> String {
        "[]".into()
    }
}

/// Longest inline `{ ... }` / `[ ... ]` before it is broken over lines.
const INLINE_WIDTH: usize = 72;

/// Renders `value` for a line indented `depth` levels. The first line of the
/// result carries no indent; continuation lines are indented absolutely.
pub(crate) fn render(syn: &dyn Syntax, value: &Bson, depth: usize) -> Result<String, CodegenError> {
    match value {
        Bson::Document(doc) => render_document(syn, doc, depth),
        Bson::Array(items) => render_array(syn, items, depth),
        Bson::Double(v) => Ok(syn.double(*v)),
        Bson::String(v) => Ok(syn.string(v)),
        Bson::Boolean(v) => Ok(syn.boolean(*v)),
        Bson::Null => Ok(syn.null()),
        Bson::Int32(v) => Ok(syn.int32(*v)),
        Bson::Int64(v) => Ok(syn.int64(*v)),
        Bson::Undefined => Ok(syn.undefined()),
        Bson::ObjectId(oid) => Ok(syn.object_id(&oid.to_hex())),
        Bson::DateTime(dt) => syn.date_time(dt.timestamp_millis()),
        Bson::Binary(Binary { subtype, bytes }) => syn.binary(*subtype, bytes),
        Bson::Decimal128(d) => Ok(syn.decimal128(&d.to_string(), d.bytes())),
        Bson::RegularExpression(r) => Ok(syn.regex(&r.pattern, &r.options)),
        Bson::Timestamp(t) => Ok(syn.timestamp(t.time, t.increment)),
        Bson::MinKey => Ok(syn.min_key()),
        Bson::MaxKey => Ok(syn.max_key()),
        Bson::JavaScriptCode(code) => Ok(syn.code(code)),
        Bson::JavaScriptCodeWithScope(c) => {
            let scope = render_document(syn, &c.scope, depth)?;
            Ok(syn.code_with_scope(&c.code, &scope))
        }
        Bson::Symbol(text) => Ok(syn.symbol(text)),
        Bson::DbPointer(_) => {
            let (namespace, id) = db_pointer_parts(value)?;
            Ok(syn.db_pointer(&namespace, &id))
        }
    }
}

/// A `DbPointer`'s fields are crate-private in `bson`; read them through its
/// canonical extended JSON form.
fn db_pointer_parts(value: &Bson) -> Result<(String, String), CodegenError> {
    let json = value.clone().into_canonical_extjson();
    let inner = json.get("$dbPointer");
    let namespace = inner.and_then(|v| v.get("$ref")).and_then(|v| v.as_str());
    let id = inner
        .and_then(|v| v.get("$id"))
        .and_then(|v| v.get("$oid"))
        .and_then(|v| v.as_str());
    match (namespace, id) {
        (Some(ns), Some(id)) => Ok((ns.to_string(), id.to_string())),
        _ => Err(CodegenError::UnsupportedValue("dbPointer")),
    }
}

/// Renders a document (shared by values, scopes and the language frames).
pub(crate) fn render_document(
    syn: &dyn Syntax,
    doc: &Document,
    depth: usize,
) -> Result<String, CodegenError> {
    if doc.is_empty() {
        return Ok(syn.empty_document());
    }
    let mut entries = Vec::with_capacity(doc.len());
    for (key, value) in doc {
        entries.push((syn.key(key), render(syn, value, depth + 1)?));
    }
    Ok(syn.document(&entries, depth))
}

/// Renders a list of documents (a pipeline).
pub(crate) fn render_documents(
    syn: &dyn Syntax,
    docs: &[Document],
    depth: usize,
) -> Result<String, CodegenError> {
    let mut entries = Vec::with_capacity(docs.len());
    for doc in docs {
        entries.push(render_document(syn, doc, depth + 1)?);
    }
    Ok(syn.pipeline(&entries, depth))
}

fn render_array(syn: &dyn Syntax, items: &[Bson], depth: usize) -> Result<String, CodegenError> {
    if items.is_empty() {
        return Ok(syn.empty_array());
    }
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        entries.push(render(syn, item, depth + 1)?);
    }
    Ok(syn.array(&entries, depth))
}

pub(crate) fn lay_out(
    syn: &dyn Syntax,
    brackets: (&str, &str),
    pad: &str,
    entries: &[String],
    depth: usize,
) -> String {
    lay_out_with(syn.indent_unit(), brackets, pad, entries, depth, true)
}

/// Inline when short, else one entry per line. `trailing_comma` puts a comma
/// after the last entry of the broken form.
pub(crate) fn lay_out_with(
    unit: &str,
    (open, close): (&str, &str),
    pad: &str,
    entries: &[String],
    depth: usize,
    trailing_comma: bool,
) -> String {
    let inline = format!("{open}{pad}{}{pad}{close}", entries.join(", "));
    if inline.len() <= INLINE_WIDTH && !entries.iter().any(|e| e.contains('\n')) {
        return inline;
    }
    let inner = indent(unit, depth + 1);
    let mut out = String::from(open);
    out.push('\n');
    for (i, entry) in entries.iter().enumerate() {
        out.push_str(&inner);
        out.push_str(entry);
        let last = i + 1 == entries.len();
        out.push_str(if last && !trailing_comma { "\n" } else { ",\n" });
    }
    out.push_str(&indent(unit, depth));
    out.push_str(close);
    out
}
