//! Editable connection form: validation, config mapping and secret persistence.
use curlybrace_core::GSSAPI_AVAILABLE;
use curlybrace_core::connection::{AuthMechanism, ConnectionConfig, SshAuth, SshConfig, TlsConfig};
use curlybrace_core::error::{CoreError, Result};
use curlybrace_core::secrets::{SecretKind, SecretStore, secret_key};
use curlybrace_core::uri::ParsedUri;
use std::fmt;
use uuid::Uuid;

const DEFAULT_SSH_PORT: u16 = 22;

/// Editable state of the connection dialog. `Debug` redacts secrets.
#[derive(Clone, PartialEq)]
pub struct ConnectionForm {
    pub id: Uuid,
    pub name: String,
    pub uri: String,
    pub auth: AuthMechanism,
    pub username: String,
    pub password: String,
    pub tls_enabled: bool,
    pub tls_ca: String,
    pub tls_client_cert: String,
    pub ssh_enabled: bool,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_user: String,
    pub ssh_key_path: String,
    pub ssh_secret: String,
    /// Explicitly delete the stored password (ignored when `password` is non-empty).
    pub clear_password: bool,
    /// Explicitly delete the stored ssh secret (ignored when `ssh_secret` is non-empty).
    pub clear_ssh_secret: bool,
    pub auth_source: String,
    pub allow_invalid_certs: bool,
    pub folder: String,
    pub created_at: Option<i64>,
    pub last_connected: Option<i64>,
}

/// What a save should do with one stored secret.
#[derive(Clone, PartialEq, Eq)]
pub enum SecretEdit {
    /// Leave the stored secret as it is.
    Unchanged,
    /// Overwrite with this value, verbatim.
    Set(String),
    /// Delete the stored secret.
    Clear,
}

impl fmt::Debug for SecretEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unchanged => f.write_str("Unchanged"),
            Self::Set(_) => f.write_str("Set(<redacted>)"),
            Self::Clear => f.write_str("Clear"),
        }
    }
}

impl SecretEdit {
    fn from_input(value: &str, clear: bool) -> Self {
        if !value.is_empty() {
            Self::Set(value.to_owned())
        } else if clear {
            Self::Clear
        } else {
            Self::Unchanged
        }
    }
}

/// Which secrets are currently stored for a connection (no values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredSecrets {
    pub password: bool,
    pub ssh_secret: bool,
}

/// Looks up which secrets exist for `id` without exposing their values.
pub fn stored_secret_flags(id: Uuid, secrets: &dyn SecretStore) -> Result<StoredSecrets> {
    let has = |kind| secrets.get(&secret_key(id, kind)).map(|v| v.is_some());
    Ok(StoredSecrets {
        password: has(SecretKind::Password)?,
        ssh_secret: has(SecretKind::SshPassword)? || has(SecretKind::SshPassphrase)?,
    })
}

impl fmt::Debug for ConnectionForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionForm")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("uri", &self.uri)
            .field("auth", &self.auth)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("tls_enabled", &self.tls_enabled)
            .field("ssh_enabled", &self.ssh_enabled)
            .field("ssh_host", &self.ssh_host)
            .field("ssh_secret", &"<redacted>")
            .field("password_edit", &self.password_edit())
            .field("ssh_secret_edit", &self.ssh_secret_edit())
            .finish_non_exhaustive()
    }
}

impl Default for ConnectionForm {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            uri: String::new(),
            auth: AuthMechanism::None,
            username: String::new(),
            password: String::new(),
            tls_enabled: false,
            tls_ca: String::new(),
            tls_client_cert: String::new(),
            ssh_enabled: false,
            ssh_host: String::new(),
            ssh_port: DEFAULT_SSH_PORT.to_string(),
            ssh_user: String::new(),
            ssh_key_path: String::new(),
            ssh_secret: String::new(),
            clear_password: false,
            clear_ssh_secret: false,
            auth_source: String::new(),
            allow_invalid_certs: false,
            folder: String::new(),
            created_at: None,
            last_connected: None,
        }
    }
}

impl ConnectionForm {
    /// Applies a parsed connection string. The password goes into the password field (it is
    /// stored in the keychain on save); the string itself is not kept.
    pub fn apply_parsed(self, parsed: ParsedUri) -> Self {
        let has_password = parsed.password.is_some();
        Self {
            uri: parsed.uri,
            username: parsed.username.unwrap_or(self.username),
            auth: parsed.auth.unwrap_or(self.auth),
            auth_source: parsed.auth_source.unwrap_or(self.auth_source),
            tls_enabled: parsed.tls.unwrap_or(self.tls_enabled),
            password: parsed
                .password
                .map_or(self.password, |p| p.expose().to_owned()),
            clear_password: self.clear_password && !has_password,
            ..self
        }
    }

    pub fn password_edit(&self) -> SecretEdit {
        SecretEdit::from_input(&self.password, self.clear_password)
    }

    pub fn ssh_secret_edit(&self) -> SecretEdit {
        SecretEdit::from_input(&self.ssh_secret, self.clear_ssh_secret)
    }

    /// Like `to_config`, plus the strict host-list check. Used when new input is stored (save);
    /// Test and Copy URI of an already-stored connection use `to_config`, which stays lenient.
    pub fn to_checked_config(&self) -> Result<ConnectionConfig> {
        let cfg = self.to_config()?;
        curlybrace_core::uri::check_hosts(&cfg.uri)?;
        Ok(cfg)
    }

    /// Validates the form and builds a config. Never includes secrets. Does not apply the strict
    /// host check, so a stored connection can still be tested or have its URI copied.
    pub fn to_config(&self) -> Result<ConnectionConfig> {
        let cfg = ConnectionConfig {
            id: self.id,
            name: self.name.trim().to_owned(),
            uri: self.uri.trim().to_owned(),
            auth: self.auth,
            username: non_blank(&self.username),
            auth_source: non_blank(&self.auth_source),
            tls: TlsConfig {
                enabled: self.tls_enabled,
                ca_file: non_blank(&self.tls_ca).map(Into::into),
                client_cert_key_file: non_blank(&self.tls_client_cert).map(Into::into),
                allow_invalid_certs: self.allow_invalid_certs,
            },
            ssh: self.ssh_config()?,
            folder: non_blank(&self.folder),
            created_at: self.created_at,
            updated_at: None,
            last_connected: self.last_connected,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    fn ssh_config(&self) -> Result<Option<SshConfig>> {
        if !self.ssh_enabled {
            return Ok(None);
        }
        let port = self
            .ssh_port
            .trim()
            .parse::<u16>()
            .map_err(|_| CoreError::Ssh("port must be a number from 1 to 65535".into()))?;
        Ok(Some(SshConfig {
            host: self.ssh_host.trim().to_owned(),
            port,
            username: self.ssh_user.trim().to_owned(),
            auth: match non_blank(&self.ssh_key_path) {
                Some(path) => SshAuth::PrivateKey { path: path.into() },
                None => SshAuth::Password,
            },
        }))
    }

    /// Form for editing `c`. Secrets are never loaded back into the form.
    pub fn from_config(c: &ConnectionConfig) -> Self {
        let path = |p: &Option<std::path::PathBuf>| {
            p.as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let mut form = Self {
            id: c.id,
            name: c.name.clone(),
            uri: c.uri.clone(),
            auth: c.auth,
            username: c.username.clone().unwrap_or_default(),
            tls_enabled: c.tls.enabled,
            auth_source: c.auth_source.clone().unwrap_or_default(),
            allow_invalid_certs: c.tls.allow_invalid_certs,
            tls_ca: path(&c.tls.ca_file),
            tls_client_cert: path(&c.tls.client_cert_key_file),
            folder: c.folder.clone().unwrap_or_default(),
            created_at: c.created_at,
            last_connected: c.last_connected,
            ..Self::default()
        };
        if let Some(ssh) = &c.ssh {
            form.ssh_enabled = true;
            form.ssh_host = ssh.host.clone();
            form.ssh_port = ssh.port.to_string();
            form.ssh_user = ssh.username.clone();
            if let SshAuth::PrivateKey { path } = &ssh.auth {
                form.ssh_key_path = path.to_string_lossy().into_owned();
            }
        }
        form
    }
}

/// Auth mechanisms offered in the UI; GSSAPI only when compiled in.
pub fn auth_options() -> Vec<AuthMechanism> {
    [
        AuthMechanism::None,
        AuthMechanism::ScramSha1,
        AuthMechanism::ScramSha256,
        AuthMechanism::X509,
        AuthMechanism::Gssapi,
        AuthMechanism::Plain,
    ]
    .into_iter()
    .filter(|a| *a != AuthMechanism::Gssapi || GSSAPI_AVAILABLE)
    .collect()
}

/// Locale key for an auth mechanism's label.
pub fn auth_label_key(a: AuthMechanism) -> &'static str {
    match a {
        AuthMechanism::None => "auth.none",
        AuthMechanism::ScramSha1 => "auth.scram_sha1",
        AuthMechanism::ScramSha256 => "auth.scram_sha256",
        AuthMechanism::X509 => "auth.x509",
        AuthMechanism::Gssapi => "auth.gssapi",
        AuthMechanism::Plain => "auth.plain",
    }
}

/// Persists the form's secrets to the store; the config never carries them.
pub fn save_connection(
    cfg: &ConnectionConfig,
    form: &ConnectionForm,
    secrets: &dyn SecretStore,
) -> Result<()> {
    let (set_kind, stale_kind) = match cfg.ssh.as_ref().map(|s| &s.auth) {
        Some(SshAuth::Password) => (Some(SecretKind::SshPassword), SecretKind::SshPassphrase),
        Some(SshAuth::PrivateKey { .. }) => {
            (Some(SecretKind::SshPassphrase), SecretKind::SshPassword)
        }
        None => (None, SecretKind::SshPassword),
    };
    write_secret(secrets, cfg, SecretKind::Password, &form.password_edit())?;
    if let Some(kind) = set_kind {
        write_secret(secrets, cfg, kind, &form.ssh_secret_edit())?;
        secrets.delete(&secret_key(cfg.id, stale_kind))
    } else {
        secrets.delete(&secret_key(cfg.id, SecretKind::SshPassword))?;
        secrets.delete(&secret_key(cfg.id, SecretKind::SshPassphrase))
    }
}

/// Applies one secret edit; `Unchanged` never touches the store.
fn write_secret(
    secrets: &dyn SecretStore,
    cfg: &ConnectionConfig,
    kind: SecretKind,
    edit: &SecretEdit,
) -> Result<()> {
    let key = secret_key(cfg.id, kind);
    match edit {
        SecretEdit::Unchanged => Ok(()),
        SecretEdit::Set(value) => secrets.set(&key, value),
        SecretEdit::Clear => secrets.delete(&key),
    }
}

fn non_blank(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::I18n;
    use curlybrace_core::connection::{SshAuth, TlsConfig};
    use curlybrace_core::secrets::{MemorySecretStore, SecretKind, secret_key};

    fn valid() -> ConnectionForm {
        ConnectionForm {
            name: "Prod ☃ Überdatenbank".into(),
            uri: "mongodb://localhost:27017".into(),
            ..ConnectionForm::default()
        }
    }

    fn ssh_form() -> ConnectionForm {
        ConnectionForm {
            ssh_enabled: true,
            ssh_host: "bastion".into(),
            ssh_port: "22".into(),
            ssh_user: "me".into(),
            ..valid()
        }
    }

    #[test]
    fn the_form_checks_the_host_list_strictly() {
        for ok in [
            "mongodb://h",
            "mongodb://h1:27017,h2:27018",
            "mongodb://[::1]:27017",
            "mongodb+srv://cluster.example.com",
            "mongodb://%2Ftmp%2Fmongodb-27017.sock",
        ] {
            let f = ConnectionForm {
                uri: ok.into(),
                ..valid()
            };
            assert!(f.to_checked_config().is_ok(), "{ok}");
        }
        for bad in ["mongodb://u:pa/ss@h", "mongodb://u@", "mongodb://h:0"] {
            let f = ConnectionForm {
                uri: bad.into(),
                ..valid()
            };
            let err = f.to_checked_config().unwrap_err();
            assert!(matches!(err, CoreError::InvalidUri(_)), "{bad}");
            assert!(!err.to_string().contains("pa/ss"), "{bad}: {err}");
        }
    }

    #[test]
    fn save_rejects_an_at_sign_in_the_tail_or_userinfo_and_a_trailing_comma_without_echo() {
        for bad in [
            "mongodb://admin:2024/S3CR3T@db",
            "mongodb://u:12345?S3CR3T@h",
            "mongodb://u:pa@S3CR3T@h",
            "mongodb://h/db@x",
            "mongodb://u@x@h",
            "mongodb://h1:27017,h2:27018,",
        ] {
            let f = ConnectionForm {
                uri: bad.into(),
                ..valid()
            };
            let err = f.to_checked_config().unwrap_err();
            assert!(matches!(err, CoreError::InvalidUri(_)), "{bad}");
            assert!(!err.to_string().contains("S3CR3T"), "{bad}: {err}");
        }
    }

    #[test]
    fn save_accepts_an_at_sign_in_a_key_value_query() {
        for ok in [
            "mongodb://h/?appName=bob@laptop",
            "mongodb://h/db?tlsCAFile=/certs@2/ca.pem",
        ] {
            let f = ConnectionForm {
                uri: ok.into(),
                ..valid()
            };
            assert!(f.to_checked_config().is_ok(), "{ok}");
        }
    }

    #[test]
    fn test_and_copy_uri_of_a_stored_bad_host_connection_still_build_a_config() {
        for stored in ["mongodb://u:pa/ss@h", "mongodb://admin:2024/S3CR3T@db"] {
            let cfg = ConnectionConfig::new("n", stored);
            let form = ConnectionForm::from_config(&cfg);
            assert!(form.to_config().is_ok(), "{stored}");
            assert!(form.to_checked_config().is_err(), "{stored}");
        }
    }

    #[test]
    fn apply_parsed_fills_fields_and_keeps_the_password_out_of_the_uri() {
        let parsed =
            curlybrace_core::uri::parse_uri("mongodb://bob:pw@h/db?authSource=admin&tls=true")
                .unwrap();
        let form = ConnectionForm {
            name: "n".into(),
            clear_password: true,
            ..ConnectionForm::default()
        }
        .apply_parsed(parsed);
        assert_eq!(form.uri, "mongodb://h/db");
        assert_eq!(form.username, "bob");
        assert_eq!(form.password, "pw");
        assert_eq!(form.auth_source, "admin");
        assert!(form.tls_enabled);
        assert!(!form.clear_password);
        assert!(form.to_config().is_ok(), "no password in the uri");
    }

    #[test]
    fn apply_parsed_without_a_password_keeps_what_the_user_typed() {
        let form = ConnectionForm {
            password: "typed".into(),
            clear_password: true,
            ..ConnectionForm::default()
        };
        let form = form.apply_parsed(curlybrace_core::uri::parse_uri("mongodb://bob@h").unwrap());
        assert_eq!(form.password, "typed");
        assert!(form.clear_password, "untouched when no password was parsed");
    }

    #[test]
    fn valid_form_maps_to_config() {
        let f = ConnectionForm {
            auth: AuthMechanism::ScramSha256,
            username: "bob".into(),
            tls_enabled: true,
            tls_ca: "/tmp/ca.pem".into(),
            ..valid()
        };
        let c = f.to_config().unwrap();
        assert_eq!(c.id, f.id);
        assert_eq!(c.name, "Prod ☃ Überdatenbank");
        assert_eq!(c.username.as_deref(), Some("bob"));
        assert_eq!(
            c.tls,
            TlsConfig {
                enabled: true,
                ca_file: Some("/tmp/ca.pem".into()),
                ..TlsConfig::default()
            }
        );
        assert!(c.ssh.is_none());
    }

    #[test]
    fn blank_name_and_bad_uri_are_rejected() {
        let f = ConnectionForm {
            name: "  ".into(),
            ..valid()
        };
        assert!(matches!(f.to_config(), Err(CoreError::InvalidName)));
        let f = ConnectionForm {
            uri: "nope".into(),
            ..valid()
        };
        assert!(matches!(f.to_config(), Err(CoreError::InvalidUri(_))));
    }

    #[test]
    fn ssh_port_must_parse_and_fields_required() {
        let f = ConnectionForm {
            ssh_port: "abc".into(),
            ..ssh_form()
        };
        assert!(matches!(f.to_config(), Err(CoreError::Ssh(_))));
        let f = ConnectionForm {
            ssh_host: " ".into(),
            ..ssh_form()
        };
        assert!(f.to_config().is_err());
        let f = ConnectionForm {
            ssh_user: "".into(),
            ..ssh_form()
        };
        assert!(f.to_config().is_err());
        let f = ConnectionForm {
            ssh_port: "0".into(),
            ..ssh_form()
        };
        assert!(f.to_config().is_err());
    }

    #[test]
    fn ssh_disabled_ignores_garbage_ssh_fields() {
        let f = ConnectionForm {
            ssh_port: "abc".into(),
            ..valid()
        };
        assert!(f.to_config().unwrap().ssh.is_none());
    }

    #[test]
    fn ssh_key_path_selects_private_key_auth() {
        let f = ConnectionForm {
            ssh_key_path: "/k/id".into(),
            ..ssh_form()
        };
        let ssh = f.to_config().unwrap().ssh.unwrap();
        assert_eq!(
            ssh.auth,
            SshAuth::PrivateKey {
                path: "/k/id".into()
            }
        );
        assert_eq!(
            ssh_form().to_config().unwrap().ssh.unwrap().auth,
            SshAuth::Password
        );
    }

    #[test]
    fn from_config_round_trips_without_secrets() {
        let f = ConnectionForm {
            auth: AuthMechanism::Plain,
            username: "bob".into(),
            password: "pw".into(),
            ssh_secret: "s".into(),
            clear_password: true,
            ssh_key_path: "/k".into(),
            ..ssh_form()
        };
        let cfg = f.to_config().unwrap();
        let back = ConnectionForm::from_config(&cfg);
        assert!(back.password.is_empty() && back.ssh_secret.is_empty());
        assert_eq!(back.password_edit(), SecretEdit::Unchanged);
        assert_eq!(back.ssh_secret_edit(), SecretEdit::Unchanged);
        assert_eq!(back.to_config().unwrap(), cfg);
    }

    #[test]
    fn debug_redacts_secrets() {
        let f = ConnectionForm {
            password: "hunter2@:/%".into(),
            ssh_secret: "sshsecret".into(),
            ..valid()
        };
        let d = format!("{f:?} {f:#?}");
        assert!(!d.contains("hunter2") && !d.contains("sshsecret"));
    }

    #[test]
    fn password_with_special_chars_round_trips_to_store_only() {
        let pw = "p@ss:w/rd%41 ü";
        let f = ConnectionForm {
            username: "bob".into(),
            password: pw.into(),
            ..valid()
        };
        let cfg = f.to_config().unwrap();
        let store = MemorySecretStore::new();
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(
            store
                .get(&secret_key(cfg.id, SecretKind::Password))
                .unwrap()
                .as_deref(),
            Some(pw)
        );
        assert!(!serde_json::to_string(&cfg).unwrap().contains(pw));
    }

    #[test]
    fn clear_password_deletes_stored_secret_and_is_idempotent() {
        let f = ConnectionForm {
            clear_password: true,
            ..valid()
        };
        let cfg = f.to_config().unwrap();
        let store = MemorySecretStore::new();
        let key = secret_key(cfg.id, SecretKind::Password);
        store.set(&key, "old").unwrap();
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
    }

    #[test]
    fn editing_without_touching_secrets_keeps_both() {
        let orig = ConnectionForm {
            password: "pw".into(),
            ssh_secret: "sp".into(),
            ..ssh_form()
        };
        let cfg = orig.to_config().unwrap();
        let store = MemorySecretStore::new();
        save_connection(&cfg, &orig, &store).unwrap();

        let mut edited = ConnectionForm::from_config(&cfg);
        edited.name = "renamed".into();
        assert_eq!(edited.password_edit(), SecretEdit::Unchanged);
        let cfg2 = edited.to_config().unwrap();
        save_connection(&cfg2, &edited, &store).unwrap();
        let get = |k| store.get(&secret_key(cfg.id, k)).unwrap();
        assert_eq!(get(SecretKind::Password).as_deref(), Some("pw"));
        assert_eq!(get(SecretKind::SshPassword).as_deref(), Some("sp"));
    }

    #[test]
    fn set_overwrites_verbatim() {
        let store = MemorySecretStore::new();
        let f = ConnectionForm {
            password: "n@w:/%41 ü ".into(),
            ..valid()
        };
        let cfg = f.to_config().unwrap();
        let key = secret_key(cfg.id, SecretKind::Password);
        store.set(&key, "old").unwrap();
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("n@w:/%41 ü "));
    }

    #[test]
    fn new_connection_without_password_stores_nothing() {
        let f = valid();
        let cfg = f.to_config().unwrap();
        let store = MemorySecretStore::new();
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(
            stored_secret_flags(cfg.id, &store).unwrap(),
            StoredSecrets::default()
        );
    }

    #[test]
    fn stored_flags_report_presence_only() {
        let store = MemorySecretStore::new();
        let id = Uuid::new_v4();
        store
            .set(&secret_key(id, SecretKind::Password), "x")
            .unwrap();
        store
            .set(&secret_key(id, SecretKind::SshPassphrase), "y")
            .unwrap();
        assert_eq!(
            stored_secret_flags(id, &store).unwrap(),
            StoredSecrets {
                password: true,
                ssh_secret: true
            }
        );
    }

    #[test]
    fn secret_edit_debug_is_redacted() {
        assert!(!format!("{:?}", SecretEdit::Set("hunter2".into())).contains("hunter2"));
    }

    #[test]
    fn auth_source_and_invalid_certs_round_trip() {
        let f = ConnectionForm {
            auth_source: "admin".into(),
            allow_invalid_certs: true,
            ..valid()
        };
        let cfg = f.to_config().unwrap();
        assert_eq!(cfg.auth_source.as_deref(), Some("admin"));
        assert!(cfg.tls.allow_invalid_certs);
        let back = ConnectionForm::from_config(&cfg);
        assert_eq!(back.auth_source, "admin");
        assert!(back.allow_invalid_certs);
        assert_eq!(back.to_config().unwrap(), cfg);
    }

    #[test]
    fn name_with_dots_and_spaces_is_accepted() {
        let f = ConnectionForm {
            name: "prod.eu west 1 ".into(),
            ..valid()
        };
        assert_eq!(f.to_config().unwrap().name, "prod.eu west 1");
    }

    #[test]
    fn ssh_secret_kind_follows_auth_type() {
        let store = MemorySecretStore::new();
        let f = ConnectionForm {
            ssh_secret: "sp@:".into(),
            ..ssh_form()
        };
        let cfg = f.to_config().unwrap();
        save_connection(&cfg, &f, &store).unwrap();
        let pw_key = secret_key(cfg.id, SecretKind::SshPassword);
        let pp_key = secret_key(cfg.id, SecretKind::SshPassphrase);
        assert_eq!(store.get(&pw_key).unwrap().as_deref(), Some("sp@:"));
        assert_eq!(store.get(&pp_key).unwrap(), None);

        let f = ConnectionForm {
            ssh_key_path: "/k".into(),
            ..f
        };
        let cfg = f.to_config().unwrap();
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(store.get(&pp_key).unwrap().as_deref(), Some("sp@:"));
        assert_eq!(store.get(&pw_key).unwrap(), None, "stale kind removed");
    }

    #[test]
    fn ssh_disabled_clears_ssh_secrets() {
        let store = MemorySecretStore::new();
        let f = valid();
        let cfg = f.to_config().unwrap();
        let k = secret_key(cfg.id, SecretKind::SshPassword);
        store.set(&k, "x").unwrap();
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(store.get(&k).unwrap(), None);
    }

    #[test]
    fn auth_options_follow_gssapi_availability() {
        let opts = auth_options();
        assert_eq!(
            opts.contains(&AuthMechanism::Gssapi),
            curlybrace_core::GSSAPI_AVAILABLE
        );
        assert!(opts.contains(&AuthMechanism::None) && opts.contains(&AuthMechanism::Plain));
    }

    #[test]
    fn every_auth_label_key_exists_in_locale() {
        let i18n = I18n::english();
        for a in [
            AuthMechanism::None,
            AuthMechanism::ScramSha1,
            AuthMechanism::ScramSha256,
            AuthMechanism::X509,
            AuthMechanism::Gssapi,
            AuthMechanism::Plain,
        ] {
            assert!(!i18n.t(auth_label_key(a)).contains('⟦'), "{a:?}");
        }
    }

    #[test]
    fn form_round_trips_folder_and_timestamps() {
        let mut c = ConnectionConfig::new("a", "mongodb://h").with_save_stamp(10);
        c.folder = Some("work".into());
        c.last_connected = Some(99);
        let back = ConnectionForm::from_config(&c).to_config().unwrap();
        assert_eq!(back.folder.as_deref(), Some("work"));
        assert_eq!(back.created_at, Some(10));
        assert_eq!(back.last_connected, Some(99));
    }

    #[test]
    fn blank_folder_text_becomes_none() {
        let mut form = ConnectionForm::from_config(&ConnectionConfig::new("a", "mongodb://h"));
        form.folder = "   ".into();
        assert_eq!(form.to_config().unwrap().folder, None);
    }
}
