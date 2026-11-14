//! BSON value syntax for the Rust driver: values live inside `bson::doc!`
//! macro bodies, with explicit constructors where the macro cannot say a type.
//! The generated code uses `?`, so it belongs in a function returning a
//! `Result` (the full snippet is one).

use std::cell::Cell;

use bson::spec::BinarySubtype;

use crate::codegen::escape::{braced_unicode_escape, double_quoted, float_text};
use crate::codegen::lossy::Conversions;
use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, lay_out_with};

/// A Rust string literal for arbitrary text.
pub(crate) fn quote(text: &str) -> String {
    double_quoted(text, braced_unicode_escape)
}

/// Entries in one document or array that `doc!` still expands under rustc's
/// default `recursion_limit` of 128 (it recurses about three times per entry).
const SAFE_MACRO_ENTRIES: usize = 32;

pub(crate) struct RustSyntax {
    /// Set when a value is spelled with a `bson::...` path.
    uses_bson_path: Cell<bool>,
    /// The most entries any one rendered document or array holds.
    widest: Cell<usize>,
    /// A DBPointer was written as its `$ref` / `$id` document.
    converted: Conversions,
}

impl RustSyntax {
    pub(crate) fn new() -> Self {
        Self {
            uses_bson_path: Cell::new(false),
            widest: Cell::new(0),
            converted: Conversions::default(),
        }
    }

    /// The `recursion_limit` the rendered macros need, when the default is
    /// not enough.
    pub(crate) fn recursion_limit(&self) -> Option<usize> {
        let widest = self.widest.get();
        (widest > SAFE_MACRO_ENTRIES).then(|| (widest * 4 + 64).next_power_of_two())
    }

    fn note_width(&self, entries: usize) {
        self.widest.set(self.widest.get().max(entries));
    }

    pub(crate) fn uses_bson_path(&self) -> bool {
        self.uses_bson_path.get()
    }

    /// Marks the `bson` module as needed and returns `text` unchanged.
    fn bson(&self, text: String) -> String {
        self.uses_bson_path.set(true);
        text
    }
}

fn owned(text: &str) -> String {
    format!("{}.to_string()", quote(text))
}

impl Syntax for RustSyntax {
    fn indent_unit(&self) -> &'static str {
        "    "
    }

    fn brace_pad(&self) -> &'static str {
        " "
    }

    fn key(&self, key: &str) -> String {
        quote(key)
    }

    fn double(&self, v: f64) -> String {
        if v.is_nan() {
            "f64::NAN".into()
        } else if v.is_infinite() {
            if v > 0.0 {
                "f64::INFINITY"
            } else {
                "f64::NEG_INFINITY"
            }
            .into()
        } else {
            float_text(v)
        }
    }

    fn string(&self, v: &str) -> String {
        quote(v)
    }

    fn int32(&self, v: i32) -> String {
        format!("{v}_i32")
    }

    fn int64(&self, v: i64) -> String {
        format!("{v}_i64")
    }

    fn boolean(&self, v: bool) -> String {
        v.to_string()
    }

    fn null(&self) -> String {
        self.bson("bson::Bson::Null".into())
    }

    fn converted(&self) -> bool {
        self.converted.happened()
    }

    fn undefined(&self) -> String {
        self.bson("bson::Bson::Undefined".into())
    }

    fn object_id(&self, hex: &str) -> String {
        self.bson(format!("bson::oid::ObjectId::parse_str(\"{hex}\")?"))
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        Ok(self.bson(format!("bson::DateTime::from_millis({millis})")))
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        let kind = match subtype {
            BinarySubtype::Generic => "bson::spec::BinarySubtype::Generic".to_string(),
            BinarySubtype::Uuid => "bson::spec::BinarySubtype::Uuid".to_string(),
            other => format!(
                "bson::spec::BinarySubtype::from(0x{:02x}_u8)",
                u8::from(other)
            ),
        };
        let items: Vec<String> = bytes.iter().map(|b| format!("0x{b:02x}")).collect();
        Ok(self.bson(format!(
            "bson::Binary {{ subtype: {kind}, bytes: vec![{}] }}",
            items.join(", ")
        )))
    }

    fn decimal128(&self, text: &str, _bytes: [u8; 16]) -> String {
        self.bson(format!("\"{text}\".parse::<bson::Decimal128>()?"))
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        self.bson(format!(
            "bson::Regex {{ pattern: {}, options: {} }}",
            owned(pattern),
            owned(options)
        ))
    }

    fn timestamp(&self, time: u32, increment: u32) -> String {
        self.bson(format!(
            "bson::Timestamp {{ time: {time}, increment: {increment} }}"
        ))
    }

    fn min_key(&self) -> String {
        self.bson("bson::Bson::MinKey".into())
    }

    fn max_key(&self) -> String {
        self.bson("bson::Bson::MaxKey".into())
    }

    fn code(&self, code: &str) -> String {
        self.bson(format!("bson::Bson::JavaScriptCode({})", owned(code)))
    }

    /// `scope` is a `{ ... }` macro body; it becomes a `doc!` here.
    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        self.bson(format!(
            "bson::Bson::JavaScriptCodeWithScope(bson::JavaScriptCodeWithScope {{ code: {}, scope: doc! {scope} }})",
            owned(code)
        ))
    }

    fn symbol(&self, text: &str) -> String {
        self.bson(format!("bson::Bson::Symbol({})", owned(text)))
    }

    /// The `bson` crate cannot build a DBPointer; it becomes the `$ref` / `$id` document.
    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        self.converted.mark();
        format!(
            "{{ \"$ref\": {}, \"$id\": {} }}",
            quote(namespace),
            self.object_id(id_hex)
        )
    }

    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        self.note_width(entries.len());
        let items: Vec<String> = entries.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        let pad = self.brace_pad();
        lay_out_with(self.indent_unit(), ("{", "}"), pad, &items, depth, true)
    }

    fn array(&self, items: &[String], depth: usize) -> String {
        self.note_width(items.len());
        lay_out_with(self.indent_unit(), ("[", "]"), "", items, depth, true)
    }

    fn pipeline(&self, items: &[String], depth: usize) -> String {
        let stages: Vec<String> = items.iter().map(|i| format!("doc! {i}")).collect();
        lay_out_with(self.indent_unit(), ("vec![", "]"), "", &stages, depth, true)
    }
}
