//! BSON value syntax for the Go driver v2 (`bson.D`, `bson.A` and the types of
//! `go.mongodb.org/mongo-driver/v2/bson`).

use std::cell::Cell;

use bson::spec::BinarySubtype;

use crate::codegen::escape::{double_quoted, float_text, unicode_escape};
use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, lay_out_with};

/// A Go interpreted string literal. `\u` is used for every escape because
/// `\xNN` in Go is a raw byte, not a code point.
pub(crate) fn quote(text: &str) -> String {
    double_quoted(text, unicode_escape)
}

pub(crate) struct GoSyntax {
    uses_math: Cell<bool>,
}

impl GoSyntax {
    pub(crate) fn new() -> Self {
        Self {
            uses_math: Cell::new(false),
        }
    }

    /// Whether `math.NaN()` / `math.Inf` appear in the rendered values.
    pub(crate) fn uses_math(&self) -> bool {
        self.uses_math.get()
    }
}

fn byte_list(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(|b| format!("0x{b:02x}")).collect();
    items.join(", ")
}

impl Syntax for GoSyntax {
    fn indent_unit(&self) -> &'static str {
        "\t"
    }

    fn brace_pad(&self) -> &'static str {
        ""
    }

    fn key(&self, key: &str) -> String {
        quote(key)
    }

    fn double(&self, v: f64) -> String {
        if v.is_nan() {
            self.uses_math.set(true);
            "math.NaN()".into()
        } else if v.is_infinite() {
            self.uses_math.set(true);
            if v > 0.0 {
                "math.Inf(1)"
            } else {
                "math.Inf(-1)"
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
        format!("int32({v})")
    }

    fn int64(&self, v: i64) -> String {
        format!("int64({v})")
    }

    fn boolean(&self, v: bool) -> String {
        v.to_string()
    }

    fn null(&self) -> String {
        "nil".into()
    }

    fn undefined(&self) -> String {
        "bson.Undefined{}".into()
    }

    fn object_id(&self, hex: &str) -> String {
        let bytes: Vec<String> = (0..hex.len())
            .step_by(2)
            .map(|i| format!("0x{}", &hex[i..i + 2]))
            .collect();
        format!("bson.ObjectID{{{}}}", bytes.join(", "))
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        Ok(format!("bson.DateTime({millis})"))
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        Ok(format!(
            "bson.Binary{{Subtype: 0x{:02x}, Data: []byte{{{}}}}}",
            u8::from(subtype),
            byte_list(bytes)
        ))
    }

    fn decimal128(&self, text: &str, bytes: [u8; 16]) -> String {
        let (mut low, mut high) = ([0_u8; 8], [0_u8; 8]);
        low.copy_from_slice(&bytes[..8]);
        high.copy_from_slice(&bytes[8..]);
        format!(
            "bson.NewDecimal128(0x{:016x}, 0x{:016x}) /* {text} */",
            u64::from_le_bytes(high),
            u64::from_le_bytes(low)
        )
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        format!(
            "bson.Regex{{Pattern: {}, Options: {}}}",
            quote(pattern),
            quote(options)
        )
    }

    fn timestamp(&self, time: u32, increment: u32) -> String {
        format!("bson.Timestamp{{T: {time}, I: {increment}}}")
    }

    fn min_key(&self) -> String {
        "bson.MinKey{}".into()
    }

    fn max_key(&self) -> String {
        "bson.MaxKey{}".into()
    }

    fn code(&self, code: &str) -> String {
        format!("bson.JavaScript({})", quote(code))
    }

    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        format!(
            "bson.CodeWithScope{{Code: {}, Scope: {scope}}}",
            quote(code)
        )
    }

    fn symbol(&self, text: &str) -> String {
        format!("bson.Symbol({})", quote(text))
    }

    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        format!(
            "bson.DBPointer{{DB: {}, Pointer: {}}}",
            quote(namespace),
            self.object_id(id_hex)
        )
    }

    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        let items: Vec<String> = entries
            .iter()
            .map(|(k, v)| format!("{{Key: {k}, Value: {v}}}"))
            .collect();
        lay_out_with(
            self.indent_unit(),
            ("bson.D{", "}"),
            "",
            &items,
            depth,
            true,
        )
    }

    fn array(&self, items: &[String], depth: usize) -> String {
        lay_out_with(self.indent_unit(), ("bson.A{", "}"), "", items, depth, true)
    }

    fn pipeline(&self, items: &[String], depth: usize) -> String {
        lay_out_with(
            self.indent_unit(),
            ("mongo.Pipeline{", "}"),
            "",
            items,
            depth,
            true,
        )
    }

    fn empty_document(&self) -> String {
        "bson.D{}".into()
    }

    fn empty_array(&self) -> String {
        "bson.A{}".into()
    }
}
