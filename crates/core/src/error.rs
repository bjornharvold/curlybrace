use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CoreError {
    #[error("invalid connection uri: {0}")]
    InvalidUri(String),
    /// A connection name is blank, too long or holds forbidden characters (never echoed).
    #[error("invalid connection name")]
    InvalidName,
    /// A folder name is too long or holds forbidden characters (never echoed).
    #[error("invalid folder name")]
    InvalidFolderName,
    #[error("invalid json: {0}")]
    InvalidJson(String),
    #[error("invalid document: {0}")]
    InvalidDocument(String),
    #[error("secret store: {0}")]
    Secret(String),
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("database: {0}")]
    Database(String),
    #[error("not found")]
    NotFound,
    /// A newer connect/disconnect made this command's session obsolete; the result was dropped.
    #[error("superseded by a newer connection request")]
    Superseded,
    /// A recorded SSH host key no longer matches the server's (possible interception).
    #[error("ssh: the host key has changed; refusing to connect")]
    SshHostKeyChanged,
    /// The server rejected the credentials, or requires authentication and none were given.
    #[error("authentication failed")]
    AuthFailed,
    /// The server refused an operation the authenticated user is not allowed to perform.
    #[error("not authorized")]
    Unauthorized,
    /// The server cannot be reached (down, wrong address, network lost). No driver text is carried.
    #[error("server unreachable")]
    Unreachable,
    /// The server answered but the TLS handshake failed (a TLS client against a plain-text
    /// server, a certificate the client rejects). No driver text is carried.
    #[error("tls handshake failed")]
    TlsHandshake,
    /// `TlsHandshake` for a session that goes through an SSH tunnel. Set by the UI.
    #[error("tls handshake through the ssh tunnel failed")]
    TlsHandshakeSsh,
    /// A network error or cleared pool on one connection: one stale socket, or a lost server.
    /// The runtime settles it with a ping of the same store, into `Unreachable` or the ordinary
    /// `Database` error carrying this text. The text is never part of the display.
    #[error("connection interrupted")]
    ConnectionInterrupted(String),
    /// `Unreachable` for a session that goes through an SSH tunnel, so the copy can name the
    /// tunnel. Set by the UI, which knows the session's endpoint; the runtime never builds it.
    #[error("server unreachable through the ssh tunnel")]
    UnreachableSsh,
    /// `Unreachable` for a write (insert, replace, delete, clone): the server may have applied
    /// it before the connection was lost. Set by the UI, which knows what the command was.
    #[error("server unreachable, the write may or may not have been applied")]
    UnreachableWrite,
    /// `UnreachableWrite` for a session that goes through an SSH tunnel.
    #[error(
        "server unreachable through the ssh tunnel, the write may or may not have been applied"
    )]
    UnreachableWriteSsh,
    /// A write (insert, replace, delete, clone) was lost to a connection error or the client-side
    /// time bound, yet the server still answers a ping: it may have applied the change. Set by
    /// the runtime, which knows the command. No driver text is carried.
    #[error("the write may or may not have been applied")]
    WriteOutcomeUnknown,
    /// TLS with certificate verification cannot be used through an SSH tunnel.
    #[error("tls with certificate verification cannot be combined with an ssh tunnel")]
    TlsThroughTunnel,
    /// Two documents on one side of a compare share a key tuple.
    #[error("compare: two documents share the same key")]
    CompareDuplicateKey,
    /// A document has no value for one of the key fields.
    #[error("compare: a document is missing a key field")]
    CompareKeyMissing,
    /// A key field holds a value that cannot serve as a key (an array or a deprecated type).
    #[error("compare: a key field holds a value that cannot be used as a key")]
    CompareKeyInvalid,
    /// A side's keyed stream was not strictly increasing: the comparator and the server disagree.
    #[error("compare: documents arrived out of key order")]
    CompareKeyOrder,
    /// No usable index covers the key and the collection is above the unindexed limit.
    #[error("compare: no usable index covers the key")]
    CompareKeyNotIndexed,
    /// The shared result budget (differences or bytes) was exhausted.
    #[error("compare: the result limit was reached")]
    CompareCapExceeded,
    /// The key or field list of a compare is not valid.
    #[error("compare: the key or field list is not valid")]
    CompareSetupInvalid,
    /// The destination is a view, or a time-series collection for a change it cannot take.
    #[error("apply: the destination cannot be written to")]
    ApplyTargetNotWritable,
    /// The server refused a write. No server text or code is carried (it can quote values).
    #[error("apply: the server refused the write")]
    ApplyWriteRejected,
    /// A write found a unique index (or `_id`) already holding the value.
    #[error("apply: a document with the same unique value already exists")]
    ApplyDuplicateKey,
    /// A write call failed as a whole (connection, write concern, malformed reply, panic): some
    /// of its operations may have been applied. No driver text is carried.
    #[error("apply: the result of the write is unknown")]
    ApplyOutcomeUnknown,
    /// An export file could not be created or written. No path or system text is carried.
    #[error("export: the file could not be written")]
    ExportWriteFailed,
    /// An export failed and the partial file could not be removed; the person must delete it.
    #[error("export: the partial file could not be removed")]
    ExportFileNotRemoved,
}

impl CoreError {
    /// Key into the UI's `en.json`. The detail text is logged, never shown raw.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::InvalidUri(_) => "errors.invalid_uri",
            Self::InvalidName => "errors.invalid_name",
            Self::InvalidFolderName => "errors.invalid_folder_name",
            Self::InvalidJson(_) => "errors.invalid_json",
            Self::InvalidDocument(_) => "errors.invalid_document",
            Self::Secret(_) => "errors.secret_store",
            Self::Ssh(_) => "errors.ssh",
            Self::Unsupported(_) => "errors.unsupported",
            Self::Database(_) => "errors.database",
            Self::NotFound => "errors.not_found",
            Self::Superseded => "errors.superseded",
            Self::AuthFailed => "errors.auth_failed",
            Self::Unauthorized => "errors.unauthorized",
            Self::Unreachable => "errors.unreachable",
            Self::TlsHandshake => "errors.tls_handshake",
            Self::TlsHandshakeSsh => "errors.tls_handshake_ssh",
            // Only seen unresolved from compare, which has no ping: it reads as unreachable.
            Self::ConnectionInterrupted(_) => "errors.unreachable",
            Self::UnreachableSsh => "errors.unreachable_ssh",
            Self::UnreachableWrite => "errors.unreachable_write",
            Self::UnreachableWriteSsh => "errors.unreachable_write_ssh",
            Self::WriteOutcomeUnknown => "errors.write_outcome_unknown",
            Self::SshHostKeyChanged => "errors.ssh_host_key_changed",
            Self::TlsThroughTunnel => "errors.tls_through_tunnel",
            Self::CompareDuplicateKey => "errors.compare_duplicate_key",
            Self::CompareKeyMissing => "errors.compare_key_missing",
            Self::CompareKeyInvalid => "errors.compare_key_invalid",
            Self::CompareKeyOrder => "errors.compare_key_order",
            Self::CompareKeyNotIndexed => "errors.compare_key_not_indexed",
            Self::CompareCapExceeded => "errors.compare_cap_exceeded",
            Self::CompareSetupInvalid => "errors.compare_setup_invalid",
            Self::ApplyTargetNotWritable => "errors.apply_target_not_writable",
            Self::ApplyWriteRejected => "errors.apply_write_rejected",
            Self::ApplyDuplicateKey => "errors.apply_duplicate_key",
            Self::ApplyOutcomeUnknown => "errors.apply_outcome_unknown",
            Self::ExportWriteFailed => "errors.export_write_failed",
            Self::ExportFileNotRemoved => "errors.export_file_not_removed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_distinct_i18n_key() {
        let errors = [
            CoreError::InvalidUri("x".into()),
            CoreError::InvalidName,
            CoreError::InvalidFolderName,
            CoreError::InvalidJson("x".into()),
            CoreError::InvalidDocument("x".into()),
            CoreError::Secret("x".into()),
            CoreError::Ssh("x".into()),
            CoreError::Unsupported("x".into()),
            CoreError::Database("x".into()),
            CoreError::NotFound,
            CoreError::Superseded,
            CoreError::AuthFailed,
            CoreError::Unauthorized,
            CoreError::Unreachable,
            CoreError::UnreachableSsh,
            CoreError::TlsHandshake,
            CoreError::TlsHandshakeSsh,
            CoreError::UnreachableWrite,
            CoreError::UnreachableWriteSsh,
            CoreError::WriteOutcomeUnknown,
            CoreError::SshHostKeyChanged,
            CoreError::TlsThroughTunnel,
            CoreError::CompareDuplicateKey,
            CoreError::CompareKeyMissing,
            CoreError::CompareKeyInvalid,
            CoreError::CompareKeyOrder,
            CoreError::CompareKeyNotIndexed,
            CoreError::CompareCapExceeded,
            CoreError::CompareSetupInvalid,
            CoreError::ApplyTargetNotWritable,
            CoreError::ApplyWriteRejected,
            CoreError::ApplyDuplicateKey,
            CoreError::ApplyOutcomeUnknown,
            CoreError::ExportWriteFailed,
            CoreError::ExportFileNotRemoved,
        ];
        let mut keys: Vec<_> = errors.iter().map(CoreError::i18n_key).collect();
        // Shares `Unreachable`'s copy: only compare can show it unresolved, and it has no ping.
        assert_eq!(
            CoreError::ConnectionInterrupted("x".into()).i18n_key(),
            CoreError::Unreachable.i18n_key()
        );
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), errors.len());
        assert!(keys.iter().all(|k| k.starts_with("errors.")));
    }

    #[test]
    fn apply_error_texts_are_static() {
        for e in [
            CoreError::ApplyWriteRejected,
            CoreError::ApplyDuplicateKey,
            CoreError::ApplyOutcomeUnknown,
        ] {
            let text = e.to_string();
            assert!(!text.is_empty() && !text.contains('{'));
        }
    }
}
