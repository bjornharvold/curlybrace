//! The range check every language applies to `skip` and `limit`, so all of them refuse the same
//! numbers with the same labelled [`CodegenError::OutOfRange`].

use crate::codegen::model::CodegenError;

/// Beyond this a JavaScript number no longer holds every integer exactly.
const JS_EXACT_MAX: u64 = 1 << 53;

/// `value` as an `i64` (the server's integer type), or the error named `name`.
pub(crate) fn i64_arg(name: &'static str, value: u64) -> Result<i64, CodegenError> {
    i64::try_from(value).map_err(|_| CodegenError::OutOfRange(name))
}

/// `value` when a JavaScript number holds it exactly (and the server accepts it).
pub(crate) fn js_arg(name: &'static str, value: u64) -> Result<u64, CodegenError> {
    match value <= JS_EXACT_MAX {
        true => Ok(value),
        false => Err(CodegenError::OutOfRange(name)),
    }
}
