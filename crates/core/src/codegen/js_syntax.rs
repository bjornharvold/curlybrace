//! BSON value syntax shared by the two JavaScript targets: the mongosh shell
//! (`NumberInt`, `ISODate`, ... helpers) and the Node.js driver (`bson` classes
//! imported from `mongodb`).

use std::cell::RefCell;
use std::collections::BTreeSet;

use bson::spec::BinarySubtype;

use crate::codegen::escape::{
    Civil, base64, civil_from_millis, double_quoted, float_text, unicode_escape, uuid_text,
};
use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, lay_out_with};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsDialect {
    Mongosh,
    Node,
}

/// The largest instant a JavaScript `Date` can hold, in milliseconds.
const JS_DATE_LIMIT_MS: i64 = 8_640_000_000_000_000;

pub(crate) struct JsSyntax {
    dialect: JsDialect,
    /// Driver classes the rendered values used (Node import list).
    used: RefCell<BTreeSet<&'static str>>,
}

impl JsSyntax {
    pub(crate) fn new(dialect: JsDialect) -> Self {
        Self {
            dialect,
            used: RefCell::new(BTreeSet::new()),
        }
    }

    /// Driver classes used so far, sorted.
    pub(crate) fn used_classes(&self) -> Vec<&'static str> {
        self.used.borrow().iter().copied().collect()
    }

    fn node(&self) -> bool {
        self.dialect == JsDialect::Node
    }

    /// Notes a driver class and returns `new Class` (Node) or `Class` (shell).
    fn ctor(&self, class: &'static str) -> String {
        if self.node() {
            self.used.borrow_mut().insert(class);
            format!("new {class}")
        } else {
            class.to_string()
        }
    }
}

fn is_plain_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// The array index a rendered key spells (`"12"`), if it is one: a canonical
/// decimal below 2^32 - 1, the keys a JavaScript object lists first.
fn array_index(rendered_key: &str) -> Option<u32> {
    let digits = rendered_key.strip_prefix('"')?.strip_suffix('"')?;
    let canonical = digits == "0" || !digits.starts_with('0');
    if !canonical || digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().filter(|&n| n != u32::MAX)
}

/// Whether a JavaScript object literal would list these keys in another order.
fn object_would_reorder(entries: &[(String, String)]) -> bool {
    let indexes: Vec<Option<u32>> = entries.iter().map(|(k, _)| array_index(k)).collect();
    let mut previous_index: Option<u32> = None;
    let mut seen_other = false;
    for index in indexes {
        match index {
            Some(n) if seen_other || previous_index.is_some_and(|p| p >= n) => return true,
            Some(n) => previous_index = Some(n),
            None => seen_other = true,
        }
    }
    false
}

/// A rendered key (`name`, `"a b"` or `["__proto__"]`) as a string literal.
fn key_literal(rendered_key: &str) -> String {
    match rendered_key
        .strip_prefix('[')
        .and_then(|k| k.strip_suffix(']'))
    {
        Some(literal) => literal.to_string(),
        None if rendered_key.starts_with('"') => rendered_key.to_string(),
        None => format!("\"{rendered_key}\""),
    }
}

/// A JavaScript string literal for arbitrary text.
pub(crate) fn quote(text: &str) -> String {
    double_quoted(text, unicode_escape)
}

impl Syntax for JsSyntax {
    fn indent_unit(&self) -> &'static str {
        "  "
    }

    fn brace_pad(&self) -> &'static str {
        " "
    }

    fn key(&self, key: &str) -> String {
        if key == "__proto__" {
            // A plain `__proto__:` key would set the prototype, not a field.
            format!("[{}]", quote(key))
        } else if is_plain_key(key) {
            key.to_string()
        } else {
            quote(key)
        }
    }

    /// An object literal, or for Node a `Map` when a plain object would list
    /// integer-like keys out of document order (the driver serializes a `Map`
    /// in insertion order). The shell has no ordered literal.
    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        if self.node() && object_would_reorder(entries) {
            let pairs: Vec<String> = entries
                .iter()
                .map(|(k, v)| format!("[{}, {v}]", key_literal(k)))
                .collect();
            let list = lay_out_with(self.indent_unit(), ("[", "]"), "", &pairs, depth, true);
            return format!("new Map({list})");
        }
        let items: Vec<String> = entries.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        let pad = self.brace_pad();
        lay_out_with(self.indent_unit(), ("{", "}"), pad, &items, depth, true)
    }

    fn double(&self, v: f64) -> String {
        if v.is_nan() {
            "NaN".into()
        } else if v.is_infinite() {
            if v > 0.0 { "Infinity" } else { "-Infinity" }.into()
        } else if v.fract() == 0.0 {
            // A bare `1` would be stored as an int; keep the double.
            format!("{}({})", self.ctor("Double"), float_text(v))
        } else {
            float_text(v)
        }
    }

    fn string(&self, v: &str) -> String {
        quote(v)
    }

    /// Both the shell and the driver store a whole JavaScript number that
    /// fits 32 bits as an int32, so no wrapper is needed.
    fn int32(&self, v: i32) -> String {
        v.to_string()
    }

    fn int64(&self, v: i64) -> String {
        if self.node() {
            self.used.borrow_mut().insert("Long");
            format!("Long.fromString(\"{v}\")")
        } else {
            format!("NumberLong(\"{v}\")")
        }
    }

    fn boolean(&self, v: bool) -> String {
        v.to_string()
    }

    fn null(&self) -> String {
        "null".into()
    }

    fn undefined(&self) -> String {
        "undefined".into()
    }

    fn object_id(&self, hex: &str) -> String {
        format!("{}(\"{hex}\")", self.ctor("ObjectId"))
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        let civil: Civil = civil_from_millis(millis);
        if civil.in_common_range() {
            let iso = civil.iso();
            return Ok(if self.node() {
                format!("new Date(\"{iso}\")")
            } else {
                format!("ISODate(\"{iso}\")")
            });
        }
        if millis.unsigned_abs() <= JS_DATE_LIMIT_MS.unsigned_abs() {
            Ok(format!("new Date({millis})"))
        } else {
            Err(CodegenError::UnsupportedValue("dateTime"))
        }
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        if subtype == BinarySubtype::Uuid {
            if let Some(uuid) = uuid_text(bytes) {
                return Ok(format!("{}(\"{uuid}\")", self.ctor("UUID")));
            }
        }
        let code = u8::from(subtype);
        let b64 = base64(bytes);
        Ok(if self.node() {
            format!(
                "{}(Buffer.from(\"{b64}\", \"base64\"), {code})",
                self.ctor("Binary")
            )
        } else {
            format!("BinData({code}, \"{b64}\")")
        })
    }

    fn decimal128(&self, text: &str, _bytes: [u8; 16]) -> String {
        if self.node() {
            self.used.borrow_mut().insert("Decimal128");
            format!("Decimal128.fromString(\"{text}\")")
        } else {
            format!("NumberDecimal(\"{text}\")")
        }
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        format!(
            "{}({}, {})",
            self.ctor("BSONRegExp"),
            quote(pattern),
            quote(options)
        )
    }

    fn timestamp(&self, time: u32, increment: u32) -> String {
        format!(
            "{}({{ t: {time}, i: {increment} }})",
            self.ctor("Timestamp")
        )
    }

    fn min_key(&self) -> String {
        format!("{}()", self.ctor("MinKey"))
    }

    fn max_key(&self) -> String {
        format!("{}()", self.ctor("MaxKey"))
    }

    fn code(&self, code: &str) -> String {
        format!("{}({})", self.ctor("Code"), quote(code))
    }

    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        format!("{}({}, {scope})", self.ctor("Code"), quote(code))
    }

    fn symbol(&self, text: &str) -> String {
        format!("{}({})", self.ctor("BSONSymbol"), quote(text))
    }

    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        format!(
            "{}({}, {}(\"{id_hex}\"))",
            self.ctor("DBRef"),
            quote(namespace),
            self.ctor("ObjectId")
        )
    }
}
