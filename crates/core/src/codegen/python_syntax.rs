//! BSON value syntax for PyMongo.

use std::cell::RefCell;
use std::collections::BTreeSet;

use bson::spec::BinarySubtype;

use crate::codegen::escape::{civil_from_millis, double_quoted, float_text, hex, x_or_u_escape};
use crate::codegen::model::CodegenError;
use crate::codegen::walk::Syntax;

pub(crate) struct PythonSyntax {
    /// Import lines the rendered values need.
    imports: RefCell<BTreeSet<&'static str>>,
}

/// A Python string literal for arbitrary text.
pub(crate) fn quote(text: &str) -> String {
    double_quoted(text, x_or_u_escape)
}

impl PythonSyntax {
    pub(crate) fn new() -> Self {
        Self {
            imports: RefCell::new(BTreeSet::new()),
        }
    }

    /// Import lines used so far, sorted.
    pub(crate) fn import_lines(&self) -> Vec<&'static str> {
        self.imports.borrow().iter().copied().collect()
    }

    fn call(&self, import: &'static str, class: &str, args: &str) -> String {
        self.imports.borrow_mut().insert(import);
        format!("{class}({args})")
    }
}

impl Syntax for PythonSyntax {
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
            "float(\"nan\")".into()
        } else if v.is_infinite() {
            if v > 0.0 {
                "float(\"inf\")"
            } else {
                "float(\"-inf\")"
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
        self.call("from bson.int64 import Int64", "Int64", &v.to_string())
    }

    fn boolean(&self, v: bool) -> String {
        if v { "True" } else { "False" }.into()
    }

    fn null(&self) -> String {
        "None".into()
    }

    fn undefined(&self) -> String {
        "None".into()
    }

    fn object_id(&self, hex: &str) -> String {
        self.call(
            "from bson.objectid import ObjectId",
            "ObjectId",
            &format!("\"{hex}\""),
        )
    }

    fn date_time(&self, millis: i64) -> Result<String, CodegenError> {
        let c = civil_from_millis(millis);
        if c.in_common_range() {
            self.imports.borrow_mut().insert("import datetime");
            return Ok(format!(
                "datetime.datetime({}, {}, {}, {}, {}, {}, {}, tzinfo=datetime.timezone.utc)",
                c.year,
                c.month,
                c.day,
                c.hour,
                c.minute,
                c.second,
                c.millis * 1000
            ));
        }
        // Beyond `datetime`'s years 1..=9999: PyMongo's millisecond wrapper.
        Ok(self.call(
            "from bson.datetime_ms import DatetimeMS",
            "DatetimeMS",
            &millis.to_string(),
        ))
    }

    fn binary(&self, subtype: BinarySubtype, bytes: &[u8]) -> Result<String, CodegenError> {
        Ok(self.call(
            "from bson.binary import Binary",
            "Binary",
            &format!("bytes.fromhex(\"{}\"), {}", hex(bytes), u8::from(subtype)),
        ))
    }

    fn decimal128(&self, text: &str, _bytes: [u8; 16]) -> String {
        self.call(
            "from bson.decimal128 import Decimal128",
            "Decimal128",
            &format!("\"{text}\""),
        )
    }

    fn regex(&self, pattern: &str, options: &str) -> String {
        self.call(
            "from bson.regex import Regex",
            "Regex",
            &format!("{}, {}", quote(pattern), quote(options)),
        )
    }

    fn timestamp(&self, time: u32, increment: u32) -> String {
        self.call(
            "from bson.timestamp import Timestamp",
            "Timestamp",
            &format!("{time}, {increment}"),
        )
    }

    fn min_key(&self) -> String {
        self.call("from bson.min_key import MinKey", "MinKey", "")
    }

    fn max_key(&self) -> String {
        self.call("from bson.max_key import MaxKey", "MaxKey", "")
    }

    fn code(&self, code: &str) -> String {
        self.call("from bson.code import Code", "Code", &quote(code))
    }

    fn code_with_scope(&self, code: &str, scope: &str) -> String {
        self.call(
            "from bson.code import Code",
            "Code",
            &format!("{}, {scope}", quote(code)),
        )
    }

    /// PyMongo cannot write the deprecated symbol type; it is sent as a string.
    fn symbol(&self, text: &str) -> String {
        quote(text)
    }

    fn db_pointer(&self, namespace: &str, id_hex: &str) -> String {
        let id = self.object_id(id_hex);
        self.call(
            "from bson.dbref import DBRef",
            "DBRef",
            &format!("{}, {id}", quote(namespace)),
        )
    }
}
