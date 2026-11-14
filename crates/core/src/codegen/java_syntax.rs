//! BSON value syntax for the Java sync driver (`org.bson.Document` trees).

use std::cell::RefCell;
use std::collections::BTreeSet;

use bson::spec::BinarySubtype;

use crate::codegen::escape::{double_quoted, float_text, unicode_escape};
use crate::codegen::model::CodegenError;
use crate::codegen::walk::{Syntax, lay_out_with};

/// Escapes for Java string literals. Java translates `\uXXXX` before it
/// lexes, so `\u000a` would end the literal: characters below U+0100 use
/// three-digit octal escapes instead, and a literal backslash in the data is
/// always doubled so it can never start a `\u` translation.
fn java_escape(code: u32) -> String {
    if code < 0x100 {
        format!("\\{code:03o}")
    } else {
        unicode_escape(code)
    }
}

/// A Java string literal for arbitrary text.
pub(crate) fn quote(text: &str) -> String {
    double_quoted(text, java_escape)
}

pub(crate) struct JavaSyntax {
    imports: RefCell<BTreeSet<&'static str>>,
}

impl JavaSyntax {
    pub(crate) fn new() -> Self {
        Self {
            imports: RefCell::new(BTreeSet::new()),
        }
    }

    /// Fully qualified names of the classes the rendered values use.
    pub(crate) fn imports(&self) -> Vec<&'static str> {
        self.imports.borrow().iter().copied().collect()
    }

    fn used(&self, class: &'static str) {
        self.imports.borrow_mut().insert(class);
    }
}

fn byte_array(bytes: &[u8]) -> String {
    let items: Vec<String> = bytes.iter().map(|b| format!("(byte) 0x{b:02x}")).collect();
    format!("new byte[] {{{}}}", items.join(", "))
}

impl Syntax for JavaSyntax {
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
            "Double.NaN".into()
        } else if v.is_infinite() {
            if v > 0.0 {
                "Double.POSITIVE_INFINITY"
            } else {
                "Double.NEGATIVE_INFINITY"
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
        "null".into()
    }

    fn undefined(&self) -> String {
        self.used("org.bson.BsonUndefined");
        "new BsonUndefined()".into()
    }

    fn object_id(&self, hex: &str) -> String {
        self.used("org.bson.types.ObjectId");
        format!("new ObjectId(\"{hex}\")")
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        self.used("java.util.Date");
        Ok(format!("new Date({millis}L)"))
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        self.used("org.bson.types.Binary");
        Ok(format!(
            "new Binary((byte) 0x{:02x}, {})",
            u8::from(subtype),
            byte_array(bytes)
        ))
    }

    fn decimal128(&self, text: &str, _bytes: [u8; 16]) -> String {
        self.used("org.bson.types.Decimal128");
        format!("Decimal128.parse(\"{text}\")")
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        self.used("org.bson.BsonRegularExpression");
        format!(
            "new BsonRegularExpression({}, {})",
            quote(pattern),
            quote(options)
        )
    }

    fn timestamp(&self, time: u32, increment: u32) -> String {
        self.used("org.bson.BsonTimestamp");
        let int = |v: u32| {
            if i32::try_from(v).is_ok() {
                v.to_string()
            } else {
                format!("(int) {v}L")
            }
        };
        format!("new BsonTimestamp({}, {})", int(time), int(increment))
    }

    fn min_key(&self) -> String {
        self.used("org.bson.types.MinKey");
        "new MinKey()".into()
    }

    fn max_key(&self) -> String {
        self.used("org.bson.types.MaxKey");
        "new MaxKey()".into()
    }

    fn code(&self, code: &str) -> String {
        self.used("org.bson.types.Code");
        format!("new Code({})", quote(code))
    }

    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        self.used("org.bson.types.CodeWithScope");
        format!("new CodeWithScope({}, {scope})", quote(code))
    }

    fn symbol(&self, text: &str) -> String {
        self.used("org.bson.types.Symbol");
        format!("new Symbol({})", quote(text))
    }

    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        self.used("org.bson.BsonDbPointer");
        let id = self.object_id(id_hex);
        format!("new BsonDbPointer({}, {id})", quote(namespace))
    }

    fn document(&self, entries: &[(String, String)], depth: usize) -> String {
        self.used("org.bson.Document");
        let Some(((first_key, first_value), rest)) = entries.split_first() else {
            return self.empty_document();
        };
        let head = format!("new Document({first_key}, {first_value})");
        let appends: Vec<String> = rest
            .iter()
            .map(|(k, v)| format!(".append({k}, {v})"))
            .collect();
        let inline = format!("{head}{}", appends.concat());
        if inline.len() <= 72 && !inline.contains('\n') {
            return inline;
        }
        let pad = self.indent_unit().repeat(depth + 1);
        let lines: String = appends.iter().map(|a| format!("\n{pad}{a}")).collect();
        format!("{head}{lines}")
    }

    fn array(&self, items: &[String], depth: usize) -> String {
        self.used("java.util.Arrays");
        // `Arrays.asList(null)` would pass a null array, not one null element.
        if items == ["null"] {
            return "Arrays.asList((Object) null)".into();
        }
        lay_out_with(
            self.indent_unit(),
            ("Arrays.asList(", ")"),
            "",
            items,
            depth,
            false,
        )
    }

    fn empty_document(&self) -> String {
        self.used("org.bson.Document");
        "new Document()".into()
    }

    fn empty_array(&self) -> String {
        self.used("java.util.Arrays");
        "Arrays.asList()".into()
    }
}
