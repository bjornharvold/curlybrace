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
    /// A server answered but the TLS handshake with it failed (no TLS on the server, a rejected
    /// certificate): the settings are wrong, not the network.
    Handshake,
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
        _ if reach == Reach::Handshake => CoreError::TlsHandshake,
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
        ErrorKind::ServerSelection { message, .. } => reach_of_selection(message),
        ErrorKind::DnsResolve { .. } => Reach::Lost,
        ErrorKind::Io(_) | ErrorKind::ConnectionPoolCleared { .. } => Reach::Suspect,
        _ => Reach::Unknown,
    }
}

/// Markers of a TLS handshake failure in the per-server reason, matched case-insensitively.
const HANDSHAKE_MARKERS: [&str; 7] = [
    "handshake",
    "peer is incompatible",
    "corrupt message",
    "peer misbehaved",
    "alert",
    "invalid peer certificate",
    "certificate",
];

/// The text the driver puts before the per-server reason in a server selection message.
const IO_REASON_PREFIX: &str = "I/O error:";

/// The reasons of a failed server selection. It quotes each server as
/// `Address: .., Tags: .., Error: Kind: I/O error: <reason>, labels: {..} }`. A reason ends at
/// its own `, labels:` (else at ` }` or the end), so the addresses, tags and replica set name
/// around it are never part of one.
fn io_reasons(message: &str) -> Vec<&str> {
    message
        .split(IO_REASON_PREFIX)
        .skip(1)
        .map(|rest| {
            let end = rest
                .find(", labels:")
                .or_else(|| rest.find(" }"))
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .collect()
}

/// A failed server selection quotes why each server failed. A TLS reason from any server means
/// a server answered but the handshake failed, even if another server was refused; any other
/// reason, or none, means it cannot be reached. Only the reasons are matched.
fn reach_of_selection(message: &str) -> Reach {
    let handshake = io_reasons(message).iter().any(|reason| {
        let reason = reason.to_lowercase();
        HANDSHAKE_MARKERS.iter().any(|m| reason.contains(m))
    });
    match handshake {
        true => Reach::Handshake,
        false => Reach::Lost,
    }
}

/// Pure: no logging. The driver's text of an unreachable server may quote addresses, so it is
/// dropped here (the typed error is static) and a failed command is logged once, by kind only,
/// where its `Failed` event is received. A suspect network error keeps only static text (the
/// I/O kind, never an address), for the ordinary database error it becomes when the server
/// still answers.
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

    /// The driver embeds the per-server reason in a server selection message.
    fn selection(reason: &str) -> String {
        format!(
            "Server selection timeout: No available servers. Topology: {{ Type: Unknown, Servers: \
             [ {{ Address: db.example:27017, Type: Unknown, Error: Kind: I/O error: {reason}, \
             labels: {{}} }}, ] }}"
        )
    }

    #[test]
    fn a_selection_failure_with_a_tls_reason_is_a_handshake_failure() {
        for reason in [
            "tls handshake eof",
            "received fatal alert: HandshakeFailure",
            "invalid peer certificate: UnknownIssuer",
            "invalid peer certificate: Expired",
            "TLS HANDSHAKE EOF",
            "peer is incompatible: NoCipherSuitesInCommon",
            "received corrupt message of type InvalidContentType",
            "peer misbehaved: IllegalSniPayload",
            "peer sent alert: UnrecognizedName",
        ] {
            assert_eq!(
                reach_of_selection(&selection(reason)),
                Reach::Handshake,
                "{reason}"
            );
        }
        assert_eq!(
            classify_parts(None, false, Reach::Handshake, "x"),
            CoreError::TlsHandshake
        );
    }

    #[test]
    fn other_selection_failures_stay_unreachable() {
        for message in [
            selection("Connection refused (os error 61)"),
            selection("failed to lookup address information: nodename nor servname provided"),
            "Server selection timeout: No available servers.".to_owned(),
            // A name that merely looks like a marker is not a reason.
            selection("Connection refused (os error 61)")
                .replace("db.example", "certificate.example"),
            // Markers before the reason do not count either.
            "tls handshake certificate Error: Kind: I/O error: Connection refused".to_owned(),
        ] {
            assert_eq!(reach_of_selection(&message), Reach::Lost, "{message}");
        }
        assert_eq!(
            classify_parts(None, false, Reach::Lost, "x"),
            CoreError::Unreachable
        );
    }

    /// Two servers in one topology, each with its own address, error and tags.
    fn two_servers(first: &str, second: &str, second_address: &str, tags: &str) -> String {
        format!(
            "Server selection timeout: No available servers. Topology: {{ Type: ReplicaSetNoPrimary, \
             Servers: [ {{ Address: a.example:27017, Type: Unknown, Error: Kind: I/O error: {first}, \
             labels: {{}} }}, {{ Address: {second_address}, Type: Unknown, Tags: {tags}, \
             Error: Kind: I/O error: {second}, labels: {{}} }}, ] }}"
        )
    }

    #[test]
    fn a_marker_outside_a_reason_does_not_make_a_handshake_failure() {
        let refused = "Connection refused (os error 61)";
        for message in [
            two_servers(refused, refused, "certificate-store.internal:27017", "{}"),
            two_servers(
                refused,
                refused,
                "b.example:27017",
                "{\"dc\": \"handshake\"}",
            ),
        ] {
            assert_eq!(reach_of_selection(&message), Reach::Lost, "{message}");
        }
    }

    #[test]
    fn a_tls_failure_on_any_server_is_a_handshake_failure() {
        let refused = "Connection refused (os error 61)";
        let tls = "received fatal alert: HandshakeFailure";
        for message in [
            two_servers(refused, tls, "b.example:27017", "{}"),
            two_servers(tls, refused, "b.example:27017", "{}"),
        ] {
            assert_eq!(reach_of_selection(&message), Reach::Handshake, "{message}");
        }
    }

    #[test]
    fn a_handshake_failure_loses_to_auth_and_has_static_text() {
        assert_eq!(
            classify_parts(None, true, Reach::Handshake, "x"),
            CoreError::AuthFailed
        );
        assert_eq!(CoreError::TlsHandshake.to_string(), "tls handshake failed");
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
