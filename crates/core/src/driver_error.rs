//! Maps MongoDB driver errors to [`CoreError`]. The only place that inspects driver error
//! kinds, so connect and every store operation classify errors the same way.
//!
//! Messages stay static for the typed variants (no server text, no secrets); anything else keeps
//! the driver's text as `Database`, as before.

use mongodb::error::{ErrorKind, WriteFailure};

use crate::error::CoreError;

/// Server error code `AuthenticationFailed`.
const CODE_AUTHENTICATION_FAILED: i32 = 18;
/// Server error code `Unauthorized`.
const CODE_UNAUTHORIZED: i32 = 13;

/// Pure classification from the facts extracted from a driver error: the server error code
/// (when it has one), whether the driver itself reported an authentication failure, and the
/// driver's text for the generic case.
pub fn classify_parts(code: Option<i32>, is_auth_kind: bool, text: &str) -> CoreError {
    match code {
        _ if is_auth_kind => CoreError::AuthFailed,
        Some(CODE_AUTHENTICATION_FAILED) => CoreError::AuthFailed,
        Some(CODE_UNAUTHORIZED) => CoreError::Unauthorized,
        _ => CoreError::Database(text.to_owned()),
    }
}

/// The server error code carried by a driver error, if any.
fn server_code(kind: &ErrorKind) -> Option<i32> {
    match kind {
        ErrorKind::Command(c) => Some(c.code),
        ErrorKind::Write(WriteFailure::WriteError(w)) => Some(w.code),
        _ => None,
    }
}

pub fn classify_driver_error(e: &mongodb::error::Error) -> CoreError {
    let is_auth = matches!(e.kind.as_ref(), ErrorKind::Authentication { .. });
    classify_parts(server_code(e.kind.as_ref()), is_auth, &e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use mongodb::error::{CommandError, Error};

    fn command_error(code: i32) -> Error {
        let cmd: CommandError = bson::from_document(doc! {
            "code": code, "codeName": "X", "errmsg": "server said no"
        })
        .expect("CommandError deserialises");
        ErrorKind::Command(cmd).into()
    }

    #[test]
    fn parts_classify_auth_unauthorized_and_the_rest() {
        assert_eq!(classify_parts(None, true, "x"), CoreError::AuthFailed);
        assert_eq!(classify_parts(Some(18), false, "x"), CoreError::AuthFailed);
        assert_eq!(
            classify_parts(Some(13), false, "x"),
            CoreError::Unauthorized
        );
        assert_eq!(
            classify_parts(Some(11000), false, "dup"),
            CoreError::Database("dup".into())
        );
        assert_eq!(
            classify_parts(None, false, "timeout"),
            CoreError::Database("timeout".into())
        );
    }

    #[test]
    fn real_driver_command_errors_are_classified_by_code() {
        assert_eq!(
            classify_driver_error(&command_error(18)),
            CoreError::AuthFailed
        );
        assert_eq!(
            classify_driver_error(&command_error(13)),
            CoreError::Unauthorized
        );
        assert!(matches!(
            classify_driver_error(&command_error(2)),
            CoreError::Database(t) if t.contains("server said no")
        ));
        let other: Error = ErrorKind::Shutdown.into();
        assert!(matches!(
            classify_driver_error(&other),
            CoreError::Database(_)
        ));
    }

    #[test]
    fn typed_auth_errors_carry_no_server_text() {
        for e in [CoreError::AuthFailed, CoreError::Unauthorized] {
            assert!(!e.to_string().contains("SCRAM"));
        }
    }
}
