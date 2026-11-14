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
    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> String;
    fn decimal128(&self, text: &str) -> String;
    fn regex(&self, pattern: &str, options: &str) -> String;
    fn timestamp(&self, time: u32, increment: u32) -> String;
    fn min_key(&self) -> String;
    fn max_key(&self) -> String;
    fn code(&self, code: &str) -> String;
    /// `scope` is the already rendered scope document.
    fn code_with_scope(&self, code: &str, scope: &str) -> String;
    fn symbol(&self, text: &str) -> String;
    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String;
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
        Bson::Binary(Binary { subtype, bytes }) => Ok(syn.binary(*subtype, bytes)),
        Bson::Decimal128(d) => Ok(syn.decimal128(&d.to_string())),
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
        return Ok("{}".into());
    }
    let mut entries = Vec::with_capacity(doc.len());
    for (key, value) in doc {
        entries.push(format!(
            "{}: {}",
            syn.key(key),
            render(syn, value, depth + 1)?
        ));
    }
    Ok(lay_out(syn, ("{", "}"), syn.brace_pad(), &entries, depth))
}

/// Renders a list of documents (a pipeline) as an array.
pub(crate) fn render_documents(
    syn: &dyn Syntax,
    docs: &[Document],
    depth: usize,
) -> Result<String, CodegenError> {
    if docs.is_empty() {
        return Ok("[]".into());
    }
    let mut entries = Vec::with_capacity(docs.len());
    for doc in docs {
        entries.push(render_document(syn, doc, depth + 1)?);
    }
    Ok(lay_out(syn, ("[", "]"), "", &entries, depth))
}

fn render_array(syn: &dyn Syntax, items: &[Bson], depth: usize) -> Result<String, CodegenError> {
    if items.is_empty() {
        return Ok("[]".into());
    }
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        entries.push(render(syn, item, depth + 1)?);
    }
    Ok(lay_out(syn, ("[", "]"), "", &entries, depth))
}

pub(crate) fn lay_out(
    syn: &dyn Syntax,
    (open, close): (&str, &str),
    pad: &str,
    entries: &[String],
    depth: usize,
) -> String {
    let inline = format!("{open}{pad}{}{pad}{close}", entries.join(", "));
    if inline.len() <= INLINE_WIDTH && !entries.iter().any(|e| e.contains('\n')) {
        return inline;
    }
    let unit = syn.indent_unit();
    let inner = indent(unit, depth + 1);
    let mut out = String::from(open);
    out.push('\n');
    for entry in entries {
        out.push_str(&inner);
        out.push_str(entry);
        out.push_str(",\n");
    }
    out.push_str(&indent(unit, depth));
    out.push_str(close);
    out
}
