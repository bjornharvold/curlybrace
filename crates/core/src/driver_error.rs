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

/// Server error code `MaxTimeMSExpired`: an operation hit its server-side time limit.
const CODE_MAX_TIME_MS_EXPIRED: i32 = 50;

/// Pure classification from the facts extracted from a driver error: the server error code
/// (when it has one), whether the driver itself reported an authentication failure, whether it
/// reported that the server cannot be reached, and the driver's text for the generic case.
pub fn classify_parts(
    code: Option<i32>,
    is_auth_kind: bool,
    is_unreachable_kind: bool,
    text: &str,
) -> CoreError {
    match code {
        _ if is_auth_kind => CoreError::AuthFailed,
        _ if is_unreachable_kind => CoreError::Unreachable,
        Some(CODE_AUTHENTICATION_FAILED) => CoreError::AuthFailed,
        Some(CODE_UNAUTHORIZED) => CoreError::Unauthorized,
        // Same value as the client-side bound reports, so the UI shows its timeout advice.
        Some(CODE_MAX_TIME_MS_EXPIRED) => CoreError::Database("timeout".into()),
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

/// Whether the driver says the server cannot be reached or the connection to it was lost:
/// no server could be selected (down, wrong address, network partition), a network I/O error
/// (refused, reset, broken pipe, connect or socket timeout), the pool of a server was cleared
/// by such an error mid-operation, or its name did not resolve. The client-side operation bound
/// is not one of these: it reports a slow operation (`Database("timeout")`), not a lost server.
fn is_unreachable_kind(kind: &ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::ServerSelection { .. }
            | ErrorKind::Io(_)
            | ErrorKind::ConnectionPoolCleared { .. }
            | ErrorKind::DnsResolve { .. }
    )
}

pub fn classify_driver_error(e: &mongodb::error::Error) -> CoreError {
    let kind = e.kind.as_ref();
    let is_auth = matches!(kind, ErrorKind::Authentication { .. });
    let unreachable = is_unreachable_kind(kind);
    if unreachable {
        // The typed error is static, and a failed TLS handshake or a replica set name mismatch
        // also surfaces as a server selection failure: keep the driver's reason in the log.
        log::warn!("driver reported the server unreachable: {e}");
    }
    classify_parts(server_code(kind), is_auth, unreachable, &e.to_string())
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
        assert_eq!(
            classify_parts(None, true, false, "x"),
            CoreError::AuthFailed
        );
        assert_eq!(
            classify_parts(Some(18), false, false, "x"),
            CoreError::AuthFailed
        );
        assert_eq!(
            classify_parts(Some(13), false, false, "x"),
            CoreError::Unauthorized
        );
        assert_eq!(
            classify_parts(Some(11000), false, false, "dup"),
            CoreError::Database("dup".into())
        );
        assert_eq!(
            classify_parts(None, false, false, "timeout"),
            CoreError::Database("timeout".into())
        );
    }

    #[test]
    fn server_time_limit_expiry_is_reported_as_a_timeout() {
        assert_eq!(
            classify_parts(Some(50), false, false, "operation exceeded time limit"),
            CoreError::Database("timeout".into())
        );
        assert_eq!(
            classify_driver_error(&command_error(50)),
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
    fn parts_classify_an_unreachable_kind_but_auth_wins() {
        assert_eq!(
            classify_parts(None, false, true, "x"),
            CoreError::Unreachable
        );
        assert_eq!(classify_parts(None, true, true, "x"), CoreError::AuthFailed);
        assert!(matches!(
            classify_parts(Some(11000), false, false, "dup"),
            CoreError::Database(_)
        ));
    }

    fn io_error(kind: std::io::ErrorKind) -> Error {
        ErrorKind::Io(std::sync::Arc::new(kind.into())).into()
    }

    #[test]
    fn network_errors_are_unreachable_whatever_their_io_kind() {
        for kind in [
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::TimedOut,
        ] {
            assert_eq!(
                classify_driver_error(&io_error(kind)),
                CoreError::Unreachable,
                "{kind:?}"
            );
        }
    }

    /// The driver's other unreachable kinds are `#[non_exhaustive]` and cannot be built here;
    /// a real server selection failure (nothing listens on port 1) covers the common one.
    #[tokio::test]
    async fn a_real_server_selection_failure_is_unreachable() {
        let options = mongodb::options::ClientOptions::parse(
            "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=300&connectTimeoutMS=300",
        )
        .await
        .expect("uri parses");
        let client = mongodb::Client::with_options(options).expect("client");
        let e = client
            .database("admin")
            .run_command(doc! { "ping": 1 })
            .await
            .expect_err("nothing listens on port 1");
        assert!(matches!(e.kind.as_ref(), ErrorKind::ServerSelection { .. }));
        assert_eq!(classify_driver_error(&e), CoreError::Unreachable);
    }

    #[test]
    fn server_replies_and_client_side_failures_are_not_unreachable() {
        for e in [command_error(2), command_error(13), command_error(18)] {
            assert_ne!(classify_driver_error(&e), CoreError::Unreachable);
        }
        let internal: Error = ErrorKind::Shutdown.into();
        assert!(matches!(
            classify_driver_error(&internal),
            CoreError::Database(_)
        ));
        // The client-side bound's own value stays a slow operation.
        assert_eq!(
            classify_driver_error(&command_error(50)),
            CoreError::Database("timeout".into())
        );
    }

    #[test]
    fn typed_auth_errors_carry_no_server_text() {
        for e in [CoreError::AuthFailed, CoreError::Unauthorized] {
            assert!(!e.to_string().contains("SCRAM"));
        }
    }
}
