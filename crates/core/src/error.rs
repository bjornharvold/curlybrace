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
        ];
        let mut keys: Vec<_> = errors.iter().map(CoreError::i18n_key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), errors.len());
        assert!(keys.iter().all(|k| k.starts_with("errors.")));
    }
}
