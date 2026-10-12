//! Which error details may be shown or logged, and how failures are logged.
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;

/// The raw text that may be shown next to the localized message: only for errors whose text is
/// the user's own input (`InvalidJson`, `InvalidDocument`) or a server message (`Database`).
/// Ssh, Secret, URI and the rest stay hidden (they are logged by the shell, never shown).
pub fn error_detail(e: &CoreError) -> Option<&str> {
    match e {
        CoreError::InvalidJson(t) | CoreError::InvalidDocument(t) | CoreError::Database(t) => {
            Some(t)
        }
        _ => None,
    }
}

/// The log line for a failed command; superseded answers are noise and are not logged.
pub fn failure_log_line(ev: &Event) -> Option<String> {
    let (context, error) = match ev {
        Event::Failed { context, error } => (*context, error),
        Event::ConnectFailed { error, .. } => ("connect", error),
        _ => return None,
    };
    (*error != CoreError::Superseded).then(|| format!("{context} failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_user_input_and_server_messages_have_a_visible_detail() {
        assert_eq!(
            error_detail(&CoreError::Database("E11000 duplicate key".into())),
            Some("E11000 duplicate key")
        );
        assert_eq!(
            error_detail(&CoreError::InvalidJson("expected value at line 1".into())),
            Some("expected value at line 1")
        );
        assert_eq!(
            error_detail(&CoreError::InvalidDocument("missing _id".into())),
            Some("missing _id")
        );
        for hidden in [
            CoreError::Ssh("secret-host".into()),
            CoreError::Secret("keychain locked".into()),
            CoreError::InvalidUri("mongodb://u:pw@h".into()),
            CoreError::Unsupported("x".into()),
            CoreError::NotFound,
            CoreError::Superseded,
            CoreError::AuthFailed,
            CoreError::Unauthorized,
            CoreError::SshHostKeyChanged,
            CoreError::TlsThroughTunnel,
        ] {
            assert_eq!(error_detail(&hidden), None, "{hidden:?}");
        }
    }

    #[test]
    fn failures_are_logged_with_context_and_error_except_superseded() {
        let find = Event::Failed {
            context: "find",
            error: CoreError::Database("boom".into()),
        };
        assert_eq!(
            failure_log_line(&find).as_deref(),
            Some("find failed: database: boom")
        );
        let connect = Event::ConnectFailed {
            epoch: 3,
            error: CoreError::AuthFailed,
        };
        assert_eq!(
            failure_log_line(&connect).as_deref(),
            Some("connect failed: authentication failed")
        );
        let superseded = Event::Failed {
            context: "find",
            error: CoreError::Superseded,
        };
        assert_eq!(failure_log_line(&superseded), None);
        assert_eq!(failure_log_line(&Event::Connected { epoch: 1 }), None);
    }

    #[tokio::test]
    async fn secret_bearing_inputs_never_reach_error_text() {
        use curlybrace_core::client::build_client_options;
        use curlybrace_core::connection::ConnectionConfig;
        const SENTINEL: &str = "S3NTINEL-pw";
        let embedded = ConnectionConfig::new("a", &format!("mongodb://u:{SENTINEL}@h:27017"));
        let query = ConnectionConfig::new(
            "b",
            &format!("mongodb://h:27017/?tlsCertificateKeyFilePassword={SENTINEL}"),
        );
        for cfg in [embedded, query] {
            let err = build_client_options(&cfg, Some(SENTINEL), None)
                .await
                .expect_err("secrets in the uri are rejected");
            let ev = Event::ConnectFailed {
                epoch: 1,
                error: err.clone(),
            };
            let line = failure_log_line(&ev).unwrap();
            assert!(!line.contains(SENTINEL) && !format!("{err:?}").contains(SENTINEL));
        }
    }
}
