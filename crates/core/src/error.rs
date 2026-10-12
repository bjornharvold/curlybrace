use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CoreError {
    #[error("invalid connection uri: {0}")]
    InvalidUri(String),
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
    /// TLS with certificate verification cannot be used through an SSH tunnel.
    #[error("tls with certificate verification cannot be combined with an ssh tunnel")]
    TlsThroughTunnel,
}

impl CoreError {
    /// Key into the UI's `en.json`. The detail text is logged, never shown raw.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::InvalidUri(_) => "errors.invalid_uri",
            Self::InvalidJson(_) => "errors.invalid_json",
            Self::InvalidDocument(_) => "errors.invalid_document",
            Self::Secret(_) => "errors.secret_store",
            Self::Ssh(_) => "errors.ssh",
            Self::Unsupported(_) => "errors.unsupported",
            Self::Database(_) => "errors.database",
            Self::NotFound => "errors.not_found",
            Self::Superseded => "errors.superseded",
            Self::SshHostKeyChanged => "errors.ssh_host_key_changed",
            Self::TlsThroughTunnel => "errors.tls_through_tunnel",
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
            CoreError::InvalidJson("x".into()),
            CoreError::InvalidDocument("x".into()),
            CoreError::Secret("x".into()),
            CoreError::Ssh("x".into()),
            CoreError::Unsupported("x".into()),
            CoreError::Database("x".into()),
            CoreError::NotFound,
            CoreError::Superseded,
            CoreError::SshHostKeyChanged,
            CoreError::TlsThroughTunnel,
        ];
        let mut keys: Vec<_> = errors.iter().map(CoreError::i18n_key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), errors.len());
        assert!(keys.iter().all(|k| k.starts_with("errors.")));
    }
}
