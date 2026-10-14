use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use uuid::Uuid;

pub const MAX_FOLDER_LEN: usize = 80;
pub const MAX_NAME_LEN: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthMechanism {
    None,
    ScramSha1,
    ScramSha256,
    X509,
    Gssapi,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub ca_file: Option<PathBuf>,
    pub client_cert_key_file: Option<PathBuf>,
    pub allow_invalid_certs: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SshAuth {
    Password,
    PrivateKey { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: Uuid,
    pub name: String,
    /// URI WITHOUT credentials; the password is stored in the Keychain.
    pub uri: String,
    pub auth: AuthMechanism,
    pub username: Option<String>,
    pub auth_source: Option<String>,
    pub tls: TlsConfig,
    pub ssh: Option<SshConfig>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub last_connected: Option<i64>,
}

impl ConnectionConfig {
    pub fn new(name: &str, uri: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            uri: uri.into(),
            auth: AuthMechanism::None,
            username: None,
            auth_source: None,
            tls: TlsConfig::default(),
            ssh: None,
            folder: None,
            created_at: None,
            updated_at: None,
            last_connected: None,
        }
    }

    pub fn with_save_stamp(self, now: i64) -> Self {
        Self {
            created_at: self.created_at.or(Some(now)),
            updated_at: Some(now),
            ..self
        }
    }

    pub fn with_connected_at(self, now: i64) -> Self {
        Self {
            last_connected: Some(now),
            ..self
        }
    }

    /// A copy with a new id and fresh timestamps. Secrets are not part of the config: the
    /// caller copies them (see `secrets::copy_all`).
    pub fn duplicate(&self, name: String, now: i64) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            created_at: Some(now),
            updated_at: Some(now),
            last_connected: None,
            ..self.clone()
        }
    }

    fn validate_folder(&self) -> Result<()> {
        let Some(folder) = &self.folder else {
            return Ok(());
        };
        let bad = folder.trim().is_empty()
            || folder.trim() != folder
            || folder.chars().count() > MAX_FOLDER_LEN
            || folder.chars().any(is_forbidden_label_char);
        if bad {
            return Err(CoreError::InvalidUri("invalid folder name".into()));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(CoreError::InvalidUri("blank connection name".into()));
        }
        if self.name.chars().count() > MAX_NAME_LEN
            || self.name.chars().any(is_forbidden_label_char)
        {
            return Err(CoreError::InvalidUri("invalid connection name".into()));
        }
        self.validate_uri()?;
        self.validate_folder()?;
        if self.auth == AuthMechanism::X509
            && !(self.tls.enabled && self.tls.client_cert_key_file.is_some())
        {
            return Err(CoreError::InvalidUri(
                "x509 requires tls and a client certificate".into(),
            ));
        }
        if let Some(ssh) = &self.ssh {
            if ssh.host.trim().is_empty() || ssh.username.trim().is_empty() || ssh.port == 0 {
                return Err(CoreError::Ssh(
                    "host, username and a non-zero port are required".into(),
                ));
            }
        }
        Ok(())
    }

    fn validate_uri(&self) -> Result<()> {
        let rest = self
            .uri
            .strip_prefix("mongodb+srv://")
            .or_else(|| self.uri.strip_prefix("mongodb://"))
            .ok_or_else(|| {
                CoreError::InvalidUri("scheme must be mongodb:// or mongodb+srv://".into())
            })?;
        let authority = rest.split(['/', '?']).next().unwrap_or("");
        if authority.is_empty() {
            return Err(CoreError::InvalidUri("missing host".into()));
        }
        // userinfo is everything before the last '@' in the authority
        if let Some((userinfo, _)) = authority.rsplit_once('@') {
            if userinfo.contains(':') {
                return Err(CoreError::InvalidUri(
                    "password must not be embedded in the uri".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Control characters plus the invisible Unicode "format" characters (bidi overrides and
/// isolates, zero-width marks, invisible operators, BOM) that can spoof a label in a list.
pub fn is_forbidden_label_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

/// `base`, or `base 2`, `base 3`, ... if a connection already has that name. A suffixed name
/// never exceeds `MAX_NAME_LEN` characters: the base is truncated (on a char boundary) to fit.
pub fn unique_name(base: &str, existing: &[ConnectionConfig]) -> String {
    let taken: HashSet<&str> = existing.iter().map(|c| c.name.as_str()).collect();
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2usize..)
        .map(|n| {
            let suffix = format!(" {n}");
            let keep = MAX_NAME_LEN.saturating_sub(suffix.chars().count());
            let head: String = base.chars().take(keep).collect();
            format!("{head}{suffix}")
        })
        .find(|candidate| !taken.contains(candidate.as_str()))
        .unwrap_or_else(|| base.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_length_and_characters_are_validated() {
        let ok = |n: &str| ConnectionConfig::new(n, "mongodb://h").validate().is_ok();
        assert!(ok(&"a".repeat(MAX_NAME_LEN)));
        assert!(!ok(&"a".repeat(MAX_NAME_LEN + 1)));
        assert!(ok(&"é".repeat(MAX_NAME_LEN)));
        assert!(ok("Prod \u{1F680} データ"));
        assert!(!ok("a\nb"));
        assert!(!ok("a\tb"));
        assert!(!ok("a\u{0}b"));
    }

    #[test]
    fn accepts_plain_and_srv_uris() {
        assert!(
            ConnectionConfig::new("a", "mongodb://localhost:27017")
                .validate()
                .is_ok()
        );
        assert!(
            ConnectionConfig::new("a", "mongodb+srv://cluster0.example.com/db")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn rejects_wrong_scheme_and_empty() {
        for bad in ["", "http://x", "localhost:27017"] {
            assert!(
                matches!(
                    ConnectionConfig::new("a", bad).validate(),
                    Err(CoreError::InvalidUri(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn rejects_embedded_password() {
        let err = ConnectionConfig::new("a", "mongodb://bob:secret@h/db")
            .validate()
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidUri(_)));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn rejects_blank_name_and_bad_ssh_port() {
        assert!(
            ConnectionConfig::new("  ", "mongodb://h")
                .validate()
                .is_err()
        );
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.ssh = Some(SshConfig {
            host: "b".into(),
            port: 0,
            username: "u".into(),
            auth: SshAuth::Password,
        });
        assert!(c.validate().is_err());
    }

    #[test]
    fn x509_needs_client_cert_and_tls() {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.auth = AuthMechanism::X509;
        assert!(c.validate().is_err());
        c.tls.enabled = true;
        c.tls.client_cert_key_file = Some("/tmp/c.pem".into());
        assert!(c.validate().is_ok());
    }

    #[test]
    fn serde_round_trip_contains_no_password_field() {
        let c = ConnectionConfig::new("a", "mongodb://h");
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.to_lowercase().contains("password"));
        assert_eq!(serde_json::from_str::<ConnectionConfig>(&json).unwrap(), c);
    }

    #[test]
    fn old_files_without_the_new_fields_still_load() {
        let old = r#"{"id":"6f9619ff-8b86-d011-b42d-00c04fc964ff","name":"a","uri":"mongodb://h",
            "auth":"None","username":null,"auth_source":null,
            "tls":{"enabled":false,"ca_file":null,"client_cert_key_file":null,"allow_invalid_certs":false},
            "ssh":null}"#;
        let c: ConnectionConfig = serde_json::from_str(old).unwrap();
        assert_eq!(c.folder, None);
        assert_eq!(c.created_at, None);
        assert_eq!(c.updated_at, None);
        assert_eq!(c.last_connected, None);
    }

    #[test]
    fn save_stamp_keeps_created_and_refreshes_updated() {
        let first = ConnectionConfig::new("a", "mongodb://h").with_save_stamp(100);
        assert_eq!((first.created_at, first.updated_at), (Some(100), Some(100)));
        let second = first.with_save_stamp(200);
        assert_eq!(
            (second.created_at, second.updated_at),
            (Some(100), Some(200))
        );
    }

    #[test]
    fn duplicate_gets_a_new_id_and_fresh_metadata_and_keeps_settings() {
        let mut src = ConnectionConfig::new("prod", "mongodb://h").with_save_stamp(10);
        src.folder = Some("work".into());
        src.last_connected = Some(50);
        src.username = Some("bob".into());
        let copy = src.duplicate("prod copy".into(), 300);
        assert_ne!(copy.id, src.id);
        assert_eq!(copy.name, "prod copy");
        assert_eq!(copy.folder.as_deref(), Some("work"));
        assert_eq!(copy.username.as_deref(), Some("bob"));
        assert_eq!((copy.created_at, copy.updated_at), (Some(300), Some(300)));
        assert_eq!(copy.last_connected, None);
    }

    #[test]
    fn unique_name_appends_a_counter_only_when_taken() {
        let a = ConnectionConfig::new("prod copy", "mongodb://h");
        let b = ConnectionConfig::new("prod copy 2", "mongodb://h");
        assert_eq!(unique_name("fresh", &[a.clone()]), "fresh");
        assert_eq!(unique_name("prod copy", &[a.clone()]), "prod copy 2");
        assert_eq!(unique_name("prod copy", &[a, b]), "prod copy 3");
    }

    #[test]
    fn folder_must_be_short_and_free_of_control_characters() {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.folder = Some("ok folder".into());
        assert!(c.validate().is_ok());
        c.folder = Some("x".repeat(MAX_FOLDER_LEN + 1));
        assert!(c.validate().is_err());
        c.folder = Some("bad\nname".into());
        assert!(c.validate().is_err());
        c.folder = Some("   ".into());
        assert!(
            c.validate().is_err(),
            "a blank folder must be None, not an empty string"
        );
    }

    const FORMAT_CHARS: [char; 5] = ['\u{202E}', '\u{200B}', '\u{2066}', '\u{FEFF}', '\u{061C}'];

    #[test]
    fn unique_name_never_exceeds_the_max_length() {
        let base = "a".repeat(MAX_NAME_LEN);
        let taken = [ConnectionConfig::new(&base, "mongodb://h")];
        let name = unique_name(&base, &taken);
        assert!(name.chars().count() <= MAX_NAME_LEN);
        assert_ne!(name, base);
        assert!(name.ends_with(" 2"));
        assert!(
            ConnectionConfig::new(&name, "mongodb://h")
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn unique_name_truncates_on_a_char_boundary() {
        let base = "é".repeat(MAX_NAME_LEN);
        let taken = [
            ConnectionConfig::new(&base, "mongodb://h"),
            ConnectionConfig::new(
                &format!("{} 2", "é".repeat(MAX_NAME_LEN - 2)),
                "mongodb://h",
            ),
        ];
        let name = unique_name(&base, &taken);
        assert!(name.chars().count() <= MAX_NAME_LEN);
        assert!(name.ends_with(" 3"));
        assert!(name.starts_with('é'));
    }

    #[test]
    fn unique_name_returns_a_free_overlong_base_unchanged() {
        let base = "a".repeat(MAX_NAME_LEN + 5);
        assert_eq!(unique_name(&base, &[]), base);
    }

    #[test]
    fn unique_name_resolves_thousands_of_collisions_quickly() {
        let mut taken: Vec<ConnectionConfig> = Vec::new();
        for _ in 0..2000 {
            let name = unique_name("a", &taken);
            taken.push(ConnectionConfig::new(&name, "mongodb://h"));
        }
        let names: std::collections::HashSet<_> = taken.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names.len(), 2000);
    }

    #[test]
    fn name_and_folder_reject_invisible_format_characters() {
        for ch in FORMAT_CHARS {
            let name = format!("pr{ch}od");
            assert!(
                ConnectionConfig::new(&name, "mongodb://h")
                    .validate()
                    .is_err()
            );
            let mut c = ConnectionConfig::new("a", "mongodb://h");
            c.folder = Some(format!("wo{ch}rk"));
            assert!(c.validate().is_err());
        }
    }

    #[test]
    fn name_and_folder_still_accept_ordinary_non_ascii() {
        for text in ["Café", "日本語", "rocket \u{1F680}"] {
            assert!(
                ConnectionConfig::new(text, "mongodb://h")
                    .validate()
                    .is_ok()
            );
            let mut c = ConnectionConfig::new("a", "mongodb://h");
            c.folder = Some(text.into());
            assert!(c.validate().is_ok());
        }
    }
}
