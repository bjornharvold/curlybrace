//! Which error details may be shown or logged, and how failures are logged.
use curlybrace_core::compare::protocol::{CompareEvent, PairOutcome};
use curlybrace_core::copy::CopyOutcome;
use curlybrace_core::copy::protocol::CopyEvent;
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
        Event::Failed { context, error, .. } | Event::RequestFailed { context, error, .. } => {
            (*context, error)
        }
        Event::ConnectFailed { error, .. } => ("connect", error),
        Event::Compare(compare) => return compare_log_line(compare),
        Event::Copy(copy) => return copy_log_line(copy),
        _ => return None,
    };
    (*error != CoreError::Superseded).then(|| format!("{context} failed: {error}"))
}

/// Compare failures are logged by context and error KIND only: the error text of a compare can
/// carry document keys or values (duplicate keys, server messages), which are never logged.
fn compare_log_line(ev: &CompareEvent) -> Option<String> {
    let (context, error) = match ev {
        CompareEvent::Failed { context, error, .. } => (*context, error),
        CompareEvent::OpenFailed { error, .. } => ("compare_open", error),
        CompareEvent::PairDone {
            outcome: PairOutcome::Failed(error),
            ..
        } => ("compare_start", error),
        _ => return None,
    };
    Some(format!("{context} failed: {}", error.i18n_key()))
}

/// Copy failures are logged by error KIND (the refusal reason is a kind too), the collection's
/// position in the paste and counts only. Never a database, collection or connection name, a
/// document key or value, or the error text: a server message can carry any of them.
fn copy_log_line(ev: &CopyEvent) -> Option<String> {
    match ev {
        CopyEvent::Failed { error, .. } if *error != CoreError::Superseded => {
            Some(format!("copy failed: {}", error.i18n_key()))
        }
        CopyEvent::ItemDone { index, report, .. } => match &report.outcome {
            // A lost connection is not a copy fault, whether it ended the run or one item.
            CopyOutcome::Failed(CoreError::Superseded) => None,
            CopyOutcome::Failed(error) => Some(format!(
                "copy_item failed: {} (collection {index})",
                error.i18n_key()
            )),
            _ => report
                .indexes
                .as_ref()
                .filter(|indexes| !indexes.failed.is_empty())
                .map(|indexes| {
                    format!(
                        "copy_indexes failed: {} index(es) (collection {index})",
                        indexes.failed.len()
                    )
                }),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::compare::key::KeyTuple;
    use curlybrace_core::compare::protocol::DuplicateReport;
    use curlybrace_core::copy::engine::{CopyFailure, CopyReport};
    use curlybrace_core::copy::structure::{IndexFailReason, IndexFailure, IndexReport};
    use curlybrace_core::error::CopyRefusal;
    use curlybrace_core::runtime::EMBEDDED_SESSION;
    use curlybrace_core::store::DuplicateGroup;
    use curlybrace_core::store::WriteFailure;

    #[test]
    fn compare_failures_are_logged_without_error_text() {
        let failed = Event::Compare(CompareEvent::Failed {
            session: 1,
            context: "compare_rows",
            error: CoreError::Database("E11000 dup key: { sku: \"SECRET-123\" }".into()),
        });
        let line = failure_log_line(&failed).unwrap();
        assert_eq!(line, "compare_rows failed: errors.database");
        let open = Event::Compare(CompareEvent::OpenFailed {
            session: 2,
            error: CoreError::Ssh("secret-host".into()),
        });
        let line = failure_log_line(&open).unwrap();
        assert!(line.starts_with("compare_open failed: "));
        assert!(!line.contains("secret-host"));
        assert_eq!(
            failure_log_line(&Event::Compare(CompareEvent::Opened { session: 1 })),
            None
        );
        let pair = Event::Compare(CompareEvent::PairDone {
            session: 1,
            run: 1,
            pair: 1,
            outcome: PairOutcome::Failed(CoreError::Database("dup { sku: \"SECRET\" }".into())),
        });
        assert_eq!(
            failure_log_line(&pair).as_deref(),
            Some("compare_start failed: errors.database")
        );
        let done = Event::Compare(CompareEvent::PairDone {
            session: 1,
            run: 1,
            pair: 1,
            outcome: PairOutcome::Completed(Default::default()),
        });
        assert_eq!(failure_log_line(&done), None);
    }

    fn item_done(outcome: CopyOutcome, indexes: Option<IndexReport>) -> Event {
        Event::Copy(CopyEvent::ItemDone {
            run: 3,
            index: 2,
            report: CopyReport {
                progress: Default::default(),
                failures: vec![CopyFailure {
                    key: bson::Bson::String("doc-key-SECRET".into()),
                    reason: WriteFailure::Rejected(121),
                }],
                outcome,
                structure: None,
                indexes,
            },
        })
    }

    #[test]
    fn copy_failures_are_logged_by_kind_without_names_keys_or_values() {
        let secret = "secret-db.secret-coll doc-key-SECRET value-SECRET hunter2";
        let failed = Event::Copy(CopyEvent::Failed {
            run: 1,
            error: CoreError::Database(secret.into()),
        });
        assert_eq!(
            failure_log_line(&failed).as_deref(),
            Some("copy failed: errors.database")
        );
        let refused = Event::Copy(CopyEvent::Failed {
            run: 1,
            error: CoreError::CopyRefused(CopyRefusal::AlreadyRunning),
        });
        assert_eq!(
            failure_log_line(&refused).as_deref(),
            Some("copy failed: errors.copy_already_running")
        );
        let item = item_done(
            CopyOutcome::Failed(CoreError::Database(secret.into())),
            None,
        );
        let line = failure_log_line(&item).unwrap();
        assert_eq!(line, "copy_item failed: errors.database (collection 2)");
        assert!(!line.contains("SECRET") && !line.contains("hunter2"));
    }

    #[test]
    fn copy_logs_failed_indexes_by_count_and_ignores_good_and_superseded_runs() {
        let bad = IndexReport {
            failed: vec![IndexFailure {
                name: "idx-SECRET".into(),
                reason: IndexFailReason::DuplicateData,
            }],
            ..IndexReport::default()
        };
        let line = failure_log_line(&item_done(CopyOutcome::Completed, Some(bad))).unwrap();
        assert_eq!(line, "copy_indexes failed: 1 index(es) (collection 2)");
        assert_eq!(
            failure_log_line(&item_done(
                CopyOutcome::Completed,
                Some(IndexReport::default())
            )),
            None
        );
        assert_eq!(
            failure_log_line(&item_done(CopyOutcome::Cancelled, None)),
            None
        );
        let superseded = Event::Copy(CopyEvent::Failed {
            run: 1,
            error: CoreError::Superseded,
        });
        assert_eq!(failure_log_line(&superseded), None);
        let lost_item = item_done(CopyOutcome::Failed(CoreError::Superseded), None);
        assert_eq!(failure_log_line(&lost_item), None);
        let done = Event::Copy(CopyEvent::Done {
            run: 1,
            cancelled: false,
        });
        assert_eq!(failure_log_line(&done), None);
    }

    #[test]
    fn duplicate_key_lookups_log_the_context_and_error_kind_only() {
        let failed = Event::Compare(CompareEvent::Failed {
            session: 1,
            context: "compare_duplicates",
            error: CoreError::Database("E11000 dup key: hunter2-secret".into()),
        });
        let line = failure_log_line(&failed).unwrap();
        assert_eq!(line, "compare_duplicates failed: errors.database");
        assert!(!line.contains("hunter2-secret"));
        let answer = Event::Compare(CompareEvent::DuplicateKeys {
            session: 1,
            pair: 1,
            source: DuplicateReport {
                groups: vec![DuplicateGroup {
                    key: KeyTuple(vec![bson::Bson::String("hunter2-secret".into())]),
                    count: 2,
                }],
                more: false,
            },
            target: DuplicateReport {
                groups: vec![],
                more: false,
            },
        });
        assert_eq!(failure_log_line(&answer), None);
    }

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
            CoreError::InvalidUri(curlybrace_core::uri_problem::UriProblem::Other),
            CoreError::InvalidName,
            CoreError::InvalidFolderName,
            CoreError::Unsupported("x".into()),
            CoreError::NotFound,
            CoreError::Superseded,
            CoreError::AuthFailed,
            CoreError::Unauthorized,
            CoreError::Unreachable,
            CoreError::TlsHandshake,
            CoreError::TlsHandshakeSsh,
            CoreError::SshHostKeyChanged,
            CoreError::TlsThroughTunnel,
            CoreError::CompareDuplicateKey,
            CoreError::CompareKeyMissing,
            CoreError::CompareKeyInvalid,
            CoreError::CompareKeyOrder,
            CoreError::CompareKeyNotIndexed,
            CoreError::CompareCapExceeded,
            CoreError::CompareSetupInvalid,
        ] {
            assert_eq!(error_detail(&hidden), None, "{hidden:?}");
        }
    }

    #[test]
    fn failures_are_logged_with_context_and_error_except_superseded() {
        let find = Event::Failed {
            session: EMBEDDED_SESSION,
            context: "find",
            error: CoreError::Database("boom".into()),
        };
        assert_eq!(
            failure_log_line(&find).as_deref(),
            Some("find failed: database: boom")
        );
        let connect = Event::ConnectFailed {
            session: EMBEDDED_SESSION,
            epoch: 3,
            error: CoreError::AuthFailed,
        };
        assert_eq!(
            failure_log_line(&connect).as_deref(),
            Some("connect failed: authentication failed")
        );
        let superseded = Event::Failed {
            session: EMBEDDED_SESSION,
            context: "find",
            error: CoreError::Superseded,
        };
        assert_eq!(failure_log_line(&superseded), None);
        assert_eq!(
            failure_log_line(&Event::Connected {
                session: EMBEDDED_SESSION,
                epoch: 1,
                topology: None,
            }),
            None
        );
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
                session: EMBEDDED_SESSION,
                epoch: 1,
                error: err.clone(),
            };
            let line = failure_log_line(&ev).unwrap();
            assert!(!line.contains(SENTINEL) && !format!("{err:?}").contains(SENTINEL));
        }
    }
}
