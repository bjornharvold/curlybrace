//! Why a connection string was refused. A small `Copy` value with one variant per cause the
//! person can fix, so the UI can say what to change. It never carries any part of the input (a
//! connection string can hold a password): every text is fixed and static.
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UriProblem {
    /// The string does not start with `mongodb://` or `mongodb+srv://`.
    MissingScheme,
    /// There is no host between the scheme (and user info) and the path.
    MissingHost,
    /// A host entry is empty or holds characters a host name cannot have.
    BadHost,
    /// A host is not plain ASCII where only ASCII is accepted (save and import do not rewrite).
    NonAsciiHost,
    /// A port is empty, not a number or outside 1-65535.
    BadPort,
    /// A password or another secret option is embedded in the string.
    SecretInUri,
    /// A raw `@` or `#` sits where it must be percent-encoded (user info, path, query): most often
    /// a password with a reserved character.
    StrayAtSign,
    /// A `%` is not followed by two hex digits, or the decoded bytes are not UTF-8.
    BadPercentEncoding,
    /// A `mongodb+srv://` string with several hosts or a port.
    SrvRules,
    /// A query option is not `key=value` or has an unsupported value.
    BadOption,
    /// A password with no user name (`:pw@host`).
    UserNeeded,
    /// Anything else (also used where the string was not typed by the person).
    Other,
}

impl UriProblem {
    pub const ALL: [UriProblem; 12] = [
        Self::MissingScheme,
        Self::MissingHost,
        Self::BadHost,
        Self::NonAsciiHost,
        Self::BadPort,
        Self::SecretInUri,
        Self::StrayAtSign,
        Self::BadPercentEncoding,
        Self::SrvRules,
        Self::BadOption,
        Self::UserNeeded,
        Self::Other,
    ];

    /// Key into the UI's `en.json`, distinct per problem.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::MissingScheme => "errors.invalid_uri_missing_scheme",
            Self::MissingHost => "errors.invalid_uri_missing_host",
            Self::BadHost => "errors.invalid_uri_bad_host",
            Self::NonAsciiHost => "errors.invalid_uri_non_ascii_host",
            Self::BadPort => "errors.invalid_uri_bad_port",
            Self::SecretInUri => "errors.invalid_uri_secret",
            Self::StrayAtSign => "errors.invalid_uri_stray_at",
            Self::BadPercentEncoding => "errors.invalid_uri_bad_percent",
            Self::SrvRules => "errors.invalid_uri_srv",
            Self::BadOption => "errors.invalid_uri_bad_option",
            Self::UserNeeded => "errors.invalid_uri_user_needed",
            Self::Other => "errors.invalid_uri",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::MissingScheme => "scheme must be mongodb:// or mongodb+srv://",
            Self::MissingHost => "missing host",
            Self::BadHost => "malformed host",
            Self::NonAsciiHost => "host is not ascii",
            Self::BadPort => "malformed port",
            Self::SecretInUri => "credentials must not be embedded in the uri",
            Self::StrayAtSign => "unexpected '@' or '#' in the connection string",
            Self::BadPercentEncoding => "malformed percent-encoding",
            Self::SrvRules => "mongodb+srv takes exactly one host and no port",
            Self::BadOption => "malformed option",
            Self::UserNeeded => "a password needs a user name",
            Self::Other => "malformed connection string",
        }
    }
}

impl fmt::Display for UriProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_problem_has_a_distinct_key_and_a_distinct_static_text() {
        let mut keys: Vec<_> = UriProblem::ALL.iter().map(|p| p.i18n_key()).collect();
        let mut texts: Vec<_> = UriProblem::ALL.iter().map(|p| p.to_string()).collect();
        keys.sort();
        keys.dedup();
        texts.sort();
        texts.dedup();
        assert_eq!(keys.len(), UriProblem::ALL.len());
        assert_eq!(texts.len(), UriProblem::ALL.len());
        assert!(keys.iter().all(|k| k.starts_with("errors.invalid_uri")));
        assert_eq!(UriProblem::Other.i18n_key(), "errors.invalid_uri");
    }
}
