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

/// What the driver's error kind says about reaching the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Nothing about reachability.
    Unknown,
    /// The server cannot be reached: no server could be selected, or its name did not resolve.
    Lost,
    /// A network error or a cleared pool on one connection. One stale socket looks like this
    /// while the server is fine, so it is only a suspicion the runtime settles with a ping.
    Suspect,
}

/// Pure classification from the facts extracted from a driver error: the server error code
/// (when it has one), whether the driver itself reported an authentication failure, what the
/// kind says about reaching the server, and the driver's text for the generic case.
pub fn classify_parts(
    code: Option<i32>,
    is_auth_kind: bool,
    reach: Reach,
    text: &str,
) -> CoreError {
    match code {
        _ if is_auth_kind => CoreError::AuthFailed,
        _ if reach == Reach::Lost => CoreError::Unreachable,
        _ if reach == Reach::Suspect => CoreError::ConnectionInterrupted(text.to_owned()),
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

/// What the driver says about reaching the server. No server could be selected (down, wrong
/// address, network partition) or its name did not resolve: the server is lost. A network I/O
/// error (refused, reset, broken pipe, socket timeout) or the pool of a server cleared by such
/// an error mid-operation may be one stale connection, so it is only suspect. The client-side
/// operation bound is neither: it reports a slow operation (`Database("timeout")`).
fn reach_of(kind: &ErrorKind) -> Reach {
    match kind {
        ErrorKind::ServerSelection { .. } | ErrorKind::DnsResolve { .. } => Reach::Lost,
        ErrorKind::Io(_) | ErrorKind::ConnectionPoolCleared { .. } => Reach::Suspect,
        _ => Reach::Unknown,
    }
}

/// Pure: no logging. The driver's text of an unreachable server may quote addresses, so it is
/// dropped here (the typed error is static) and a failed command is logged once, by kind only,
/// where its `Failed` event is received. A suspect network error keeps the text, for the
/// ordinary database error it becomes when the server still answers.
pub fn classify_driver_error(e: &mongodb::error::Error) -> CoreError {
    let kind = e.kind.as_ref();
    let is_auth = matches!(kind, ErrorKind::Authentication { .. });
    classify_parts(server_code(kind), is_auth, reach_of(kind), &text_of(e))
}

/// The text a classified error may carry. A network error or a cleared pool names the server
/// (`Connection pool for host:port cleared ...`, or an I/O message quoting the peer), and that
/// text survives into `Database(text)` once a ping shows the server alive, so only a static
/// description is kept for them (`ConnectionPoolCleared` is `#[non_exhaustive]`, so a test cannot
/// build it). Any other kind keeps the driver's text.
fn text_of(e: &mongodb::error::Error) -> String {
    match e.kind.as_ref() {
        ErrorKind::Io(io) => io.kind().to_string(),
        ErrorKind::ConnectionPoolCleared { .. } => "connection pool cleared".to_owned(),
        _ => e.to_string(),
    }
}

/// For a connect, which has no open session to ping: a suspect connection error means the server
/// could not be reached.
pub fn classify_connect_error(e: &mongodb::error::Error) -> CoreError {
    match classify_driver_error(e) {
        CoreError::ConnectionInterrupted(_) => CoreError::Unreachable,
        other => other,
    }
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
            classify_parts(None, true, Reach::Unknown, "x"),
            CoreError::AuthFailed
        );
        assert_eq!(
            classify_parts(Some(18), false, Reach::Unknown, "x"),
            CoreError::AuthFailed
        );
        assert_eq!(
            classify_parts(Some(13), false, Reach::Unknown, "x"),
            CoreError::Unauthorized
        );
        assert_eq!(
            classify_parts(Some(11000), false, Reach::Unknown, "dup"),
            CoreError::Database("dup".into())
        );
        assert_eq!(
            classify_parts(None, false, Reach::Unknown, "timeout"),
            CoreError::Database("timeout".into())
        );
    }

    #[test]
    fn server_time_limit_expiry_is_reported_as_a_timeout() {
        assert_eq!(
            classify_parts(
                Some(50),
                false,
                Reach::Unknown,
                "operation exceeded time limit"
            ),
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
            classify_parts(None, false, Reach::Lost, "x"),
            CoreError::Unreachable
        );
        assert_eq!(
            classify_parts(None, true, Reach::Lost, "x"),
            CoreError::AuthFailed
        );
        assert_eq!(
            classify_parts(None, false, Reach::Suspect, "reset"),
            CoreError::ConnectionInterrupted("reset".into())
        );
        assert!(matches!(
            classify_parts(Some(11000), false, Reach::Unknown, "dup"),
            CoreError::Database(_)
        ));
    }

    fn io_error(kind: std::io::ErrorKind) -> Error {
        ErrorKind::Io(std::sync::Arc::new(kind.into())).into()
    }

    #[test]
    fn network_errors_are_only_suspect_whatever_their_io_kind() {
        for kind in [
            std::io::ErrorKind::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::UnexpectedEof,
            std::io::ErrorKind::TimedOut,
        ] {
            let classified = classify_driver_error(&io_error(kind));
            assert!(
                matches!(classified, CoreError::ConnectionInterrupted(_)),
                "{kind:?}"
            );
            // A connect has no session to ping: there it is unreachable.
            assert_eq!(
                classify_connect_error(&io_error(kind)),
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
        assert_eq!(classify_connect_error(&e), CoreError::Unreachable);
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
    fn a_network_error_keeps_no_address_in_its_text() {
        let quoted = std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection to 10.0.0.5:27017 reset",
        );
        let e: Error = ErrorKind::Io(std::sync::Arc::new(quoted)).into();
        let CoreError::ConnectionInterrupted(text) = classify_driver_error(&e) else {
            panic!("a network error is suspect");
        };
        assert!(
            !text.contains("10.0.0.5") && !text.contains("27017"),
            "{text}"
        );
        assert!(!text.is_empty());
    }

    #[test]
    fn typed_auth_errors_carry_no_server_text() {
        for e in [CoreError::AuthFailed, CoreError::Unauthorized] {
            assert!(!e.to_string().contains("SCRAM"));
        }
    }
}
