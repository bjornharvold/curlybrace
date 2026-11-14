//! BSON value syntax for the PHP library (`MongoDB\BSON\*` classes, arrays for
//! documents). Strings are single-quoted unless they hold invisible characters
//! (see `single_or_double_quoted`), so nothing is ever interpolated.

use bson::spec::BinarySubtype;

use crate::codegen::escape::{float_text, hex, single_or_double_quoted};
use crate::codegen::lossy::Conversions;
use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, lay_out_with};

/// A PHP string literal for arbitrary text.
pub(crate) fn quote(text: &str) -> String {
    single_or_double_quoted(text, '$')
}

pub(crate) struct PhpSyntax {
    /// A value was converted (`undefined` -> `null`, a symbol -> a string, DBPointer -> array).
    converted: Conversions,
}

impl PhpSyntax {
    pub(crate) fn new() -> Self {
        Self {
            converted: Conversions::default(),
        }
    }
}

/// Rendered keys PHP would turn into integers (`'0'`, `'12'`, `'-3'`).
fn is_integer_key(rendered: &str) -> bool {
    let Some(inner) = rendered
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
    else {
        return false;
    };
    let digits = inner.strip_prefix('-').unwrap_or(inner);
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
        && inner != "-0"
}

/// An integer as PHP source: a string for the one value PHP reads as a float.
fn int_arg(v: i64) -> String {
    if v == i64::MIN {
        format!("'{v}'")
    } else {
        v.to_string()
    }
}

impl Syntax for PhpSyntax {
    fn indent_unit(&self) -> &'static str {
        "    "
    }

    fn brace_pad(&self) -> &'static str {
        ""
    }

    fn key(&self, key: &str) -> String {
        quote(key)
    }

    fn double(&self, v: f64) -> String {
        if v.is_nan() {
            "NAN".into()
        } else if v.is_infinite() {
            if v > 0.0 { "INF" } else { "-INF" }.into()
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

    /// Always explicit: a plain PHP int that fits 32 bits is stored as int32.
    fn int64(&self, v: i64) -> String {
        format!("new MongoDB\\BSON\\Int64('{v}')")
    }

    fn boolean(&self, v: bool) -> String {
        v.to_string()
    }

    fn null(&self) -> String {
        "null".into()
    }

    /// The library cannot build `undefined`; it is sent as null.
    fn undefined(&self) -> String {
        self.converted.mark();
        "null".into()
    }

    fn converted(&self) -> bool {
        self.converted.happened()
    }

    fn object_id(&self, hex: &str) -> String {
        format!("new MongoDB\\BSON\\ObjectId('{hex}')")
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        Ok(format!(
            "new MongoDB\\BSON\\UTCDateTime({})",
            int_arg(millis)
        ))
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        Ok(format!(
            "new MongoDB\\BSON\\Binary(hex2bin('{}'), {})",
            hex(bytes),
            u8::from(subtype)
        ))
    }

    fn decimal128(&self, text: &str, _bytes: [u8; 16]) -> String {
        format!("new MongoDB\\BSON\\Decimal128('{text}')")
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        format!(
            "new MongoDB\\BSON\\Regex({}, {})",
            quote(pattern),
            quote(options)
        )
    }

    /// The PHP class takes `(increment, timestamp)`.
    fn timestamp(&self, time: u32, increment: u32) -> String {
        format!("new MongoDB\\BSON\\Timestamp({increment}, {time})")
    }

    fn min_key(&self) -> String {
        "new MongoDB\\BSON\\MinKey()".into()
    }

    fn max_key(&self) -> String {
        "new MongoDB\\BSON\\MaxKey()".into()
    }

    fn code(&self, code: &str) -> String {
        format!("new MongoDB\\BSON\\Javascript({})", quote(code))
    }

    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        format!("new MongoDB\\BSON\\Javascript({}, {scope})", quote(code))
    }

    /// The library cannot build the deprecated symbol type; it is a string.
    fn symbol(&self, text: &str) -> String {
        self.converted.mark();
        quote(text)
    }

    /// Sent as the equivalent `$ref` / `$id` document.
    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        self.converted.mark();
        format!(
            "['$ref' => {}, '$id' => {}]",
            quote(namespace),
            self.object_id(id_hex)
        )
    }

    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        let items: Vec<String> = entries.iter().map(|(k, v)| format!("{k} => {v}")).collect();
        let list = lay_out_with(self.indent_unit(), ("[", "]"), "", &items, depth, true);
        // Integer-like keys would make PHP read the array as a list or renumber it.
        if entries.iter().any(|(k, _)| is_integer_key(k)) {
            format!("(object) {list}")
        } else {
            list
        }
    }

    fn empty_document(&self) -> String {
        "(object) []".into()
    }
}
