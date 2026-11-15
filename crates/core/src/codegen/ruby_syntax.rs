//! BSON value syntax for the Ruby driver (`BSON::*` classes, hashes for
//! documents). Strings are single-quoted unless they hold invisible
//! characters (see `single_or_double_quoted`), so `#{}` never interpolates.

use bson::spec::BinarySubtype;

use crate::codegen::escape::{float_text, hex, single_or_double_quoted};
use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, lay_out_with};

/// A Ruby string literal for arbitrary text.
pub(crate) fn quote(text: &str) -> String {
    single_or_double_quoted(text, '#')
}

pub(crate) struct RubySyntax;

/// The symbol `BSON::Binary` takes for a subtype, if it has one.
fn subtype_symbol(subtype: BinarySubtype) -> Option<&'static str> {
    match u8::from(subtype) {
        0 => Some(":generic"),
        1 => Some(":function"),
        2 => Some(":old"),
        3 => Some(":uuid_old"),
        4 => Some(":uuid"),
        5 => Some(":md5"),
        6 => Some(":ciphertext"),
        7 => Some(":column"),
        8 => Some(":sensitive"),
        0x80 => Some(":user"),
        _ => None,
    }
}

impl Syntax for RubySyntax {
    fn indent_unit(&self) -> &'static str {
        "  "
    }

    fn brace_pad(&self) -> &'static str {
        " "
    }

    fn key(&self, key: &str) -> String {
        quote(key)
    }

    fn double(&self, v: f64) -> String {
        if v.is_nan() {
            "Float::NAN".into()
        } else if v.is_infinite() {
            if v > 0.0 {
                "Float::INFINITY"
            } else {
                "-Float::INFINITY"
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

    /// Always explicit: a plain Ruby Integer that fits 32 bits is stored as int32.
    fn int64(&self, v: i64) -> String {
        format!("BSON::Int64.new({v})")
    }

    fn boolean(&self, v: bool) -> String {
        v.to_string()
    }

    fn null(&self) -> String {
        "nil".into()
    }

    fn undefined(&self) -> String {
        "BSON::Undefined.new".into()
    }

    fn object_id(&self, hex: &str) -> String {
        format!("BSON::ObjectId.from_string('{hex}')")
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        let seconds = millis.div_euclid(1000);
        let rest = millis.rem_euclid(1000);
        Ok(format!("Time.at({seconds}, {rest}, :millisecond).utc"))
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        let symbol = subtype_symbol(subtype).ok_or(CodegenError::UnsupportedValue("binary"))?;
        Ok(format!(
            "BSON::Binary.new(['{}'].pack('H*'), {symbol})",
            hex(bytes)
        ))
    }

    fn decimal128(&self, text: &str, _bytes: [u8; 16]) -> String {
        format!("BSON::Decimal128.new('{text}')")
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        format!(
            "BSON::Regexp::Raw.new({}, {})",
            quote(pattern),
            quote(options)
        )
    }

    fn timestamp(&self, time: u32, increment: u32) -> String {
        format!("BSON::Timestamp.new({time}, {increment})")
    }

    fn min_key(&self) -> String {
        "BSON::MinKey.new".into()
    }

    fn max_key(&self) -> String {
        "BSON::MaxKey.new".into()
    }

    fn code(&self, code: &str) -> String {
        format!("BSON::Code.new({})", quote(code))
    }

    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        format!("BSON::CodeWithScope.new({}, {scope})", quote(code))
    }

    fn symbol(&self, text: &str) -> String {
        format!("BSON::Symbol::Raw.new({})", quote(text))
    }

    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        format!(
            "BSON::DbPointer.new({}, {})",
            quote(namespace),
            self.object_id(id_hex)
        )
    }

    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        let items: Vec<String> = entries.iter().map(|(k, v)| format!("{k} => {v}")).collect();
        lay_out_with(self.indent_unit(), ("{", "}"), " ", &items, depth, true)
    }
}
