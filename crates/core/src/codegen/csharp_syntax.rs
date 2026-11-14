//! BSON value syntax for the .NET driver (`BsonDocument` / `BsonArray`).
//!
//! Strings are always regular (never interpolated `$""`) literals.

use bson::spec::BinarySubtype;

use crate::codegen::escape::{double_quoted, float_text, unicode_escape};
use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, lay_out_with};

/// A C# string literal for arbitrary text (`\uXXXX` is fixed width there).
pub(crate) fn quote(text: &str) -> String {
    double_quoted(text, unicode_escape)
}

pub(crate) struct CsharpSyntax;

fn byte_array(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "new byte[0]".into();
    }
    let items: Vec<String> = bytes.iter().map(|b| format!("0x{b:02x}")).collect();
    format!("new byte[] {{ {} }}", items.join(", "))
}

fn subtype_text(subtype: BinarySubtype) -> String {
    match u8::from(subtype) {
        0 => "BsonBinarySubType.Binary".into(),
        4 => "BsonBinarySubType.UuidStandard".into(),
        other => format!("(BsonBinarySubType)0x{other:02x}"),
    }
}

impl Syntax for CsharpSyntax {
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
            "double.NaN".into()
        } else if v.is_infinite() {
            if v > 0.0 {
                "double.PositiveInfinity"
            } else {
                "double.NegativeInfinity"
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
        v.to_string()
    }

    fn int64(&self, v: i64) -> String {
        format!("{v}L")
    }

    fn boolean(&self, v: bool) -> String {
        v.to_string()
    }

    fn null(&self) -> String {
        "BsonNull.Value".into()
    }

    fn undefined(&self) -> String {
        "BsonUndefined.Value".into()
    }

    fn object_id(&self, hex: &str) -> String {
        format!("new ObjectId(\"{hex}\")")
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        Ok(format!("new BsonDateTime({millis}L)"))
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        Ok(format!(
            "new BsonBinaryData({}, {})",
            byte_array(bytes),
            subtype_text(subtype)
        ))
    }

    fn decimal128(&self, text: &str, _bytes: [u8; 16]) -> String {
        format!("Decimal128.Parse(\"{text}\")")
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        format!(
            "new BsonRegularExpression({}, {})",
            quote(pattern),
            quote(options)
        )
    }

    fn timestamp(&self, time: u32, increment: u32) -> String {
        let int = |v: u32| {
            if i32::try_from(v).is_ok() {
                v.to_string()
            } else {
                format!("unchecked((int){v}L)")
            }
        };
        format!("new BsonTimestamp({}, {})", int(time), int(increment))
    }

    fn min_key(&self) -> String {
        "BsonMinKey.Value".into()
    }

    fn max_key(&self) -> String {
        "BsonMaxKey.Value".into()
    }

    fn code(&self, code: &str) -> String {
        format!("new BsonJavaScript({})", quote(code))
    }

    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        format!("new BsonJavaScriptWithScope({}, {scope})", quote(code))
    }

    fn symbol(&self, text: &str) -> String {
        format!("BsonSymbolTable.Lookup({})", quote(text))
    }

    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        format!(
            "new BsonDbPointer({}, {})",
            quote(namespace),
            self.object_id(id_hex)
        )
    }

    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        let items: Vec<String> = entries
            .iter()
            .map(|(k, v)| format!("{{ {k}, {v} }}"))
            .collect();
        lay_out_with(
            self.indent_unit(),
            ("new BsonDocument {", "}"),
            " ",
            &items,
            depth,
            true,
        )
    }

    fn array(&self, items: &[String], depth: usize) -> String {
        lay_out_with(
            self.indent_unit(),
            ("new BsonArray {", "}"),
            " ",
            items,
            depth,
            true,
        )
    }

    fn pipeline(&self, items: &[String], depth: usize) -> String {
        lay_out_with(
            self.indent_unit(),
            ("new[] {", "}"),
            " ",
            items,
            depth,
            true,
        )
    }

    fn empty_document(&self) -> String {
        "new BsonDocument()".into()
    }

    fn empty_array(&self) -> String {
        "new BsonArray()".into()
    }
}
