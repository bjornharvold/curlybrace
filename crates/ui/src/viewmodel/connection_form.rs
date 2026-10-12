//! Editable connection form: validation, config mapping and secret persistence.
use mongostudio_core::GSSAPI_AVAILABLE;
use mongostudio_core::connection::{
    AuthMechanism, ConnectionConfig, SshAuth, SshConfig, TlsConfig,
};
use mongostudio_core::error::{CoreError, Result};
use mongostudio_core::secrets::{SecretKind, SecretStore, secret_key};
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
        }
    }
}

impl ConnectionForm {
    /// Validates the form and builds a config. Never includes secrets.
    pub fn to_config(&self) -> Result<ConnectionConfig> {
        let cfg = ConnectionConfig {
            id: self.id,
            name: self.name.trim().to_owned(),
            uri: self.uri.trim().to_owned(),
            auth: self.auth,
            username: non_blank(&self.username),
            auth_source: None,
            tls: TlsConfig {
                enabled: self.tls_enabled,
                ca_file: non_blank(&self.tls_ca).map(Into::into),
                client_cert_key_file: non_blank(&self.tls_client_cert).map(Into::into),
                allow_invalid_certs: false,
            },
            ssh: self.ssh_config()?,
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
            tls_ca: path(&c.tls.ca_file),
            tls_client_cert: path(&c.tls.client_cert_key_file),
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
    write_secret(secrets, cfg, SecretKind::Password, &form.password)?;
    if let Some(kind) = set_kind {
        write_secret(secrets, cfg, kind, &form.ssh_secret)?;
        secrets.delete(&secret_key(cfg.id, stale_kind))
    } else {
        secrets.delete(&secret_key(cfg.id, SecretKind::SshPassword))?;
        secrets.delete(&secret_key(cfg.id, SecretKind::SshPassphrase))
    }
}

/// Stores a non-empty secret verbatim; an empty one removes the entry.
fn write_secret(
    secrets: &dyn SecretStore,
    cfg: &ConnectionConfig,
    kind: SecretKind,
    value: &str,
) -> Result<()> {
    let key = secret_key(cfg.id, kind);
    if value.is_empty() {
        secrets.delete(&key)
    } else {
        secrets.set(&key, value)
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
    use mongostudio_core::connection::{SshAuth, TlsConfig};
    use mongostudio_core::secrets::{MemorySecretStore, SecretKind, secret_key};

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
        assert!(f.to_config().is_err());
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
            ssh_key_path: "/k".into(),
            ..ssh_form()
        };
        let cfg = f.to_config().unwrap();
        let back = ConnectionForm::from_config(&cfg);
        assert!(back.password.is_empty() && back.ssh_secret.is_empty());
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
    fn empty_password_deletes_stored_secret() {
        let f = valid();
        let cfg = f.to_config().unwrap();
        let store = MemorySecretStore::new();
        let key = secret_key(cfg.id, SecretKind::Password);
        store.set(&key, "old").unwrap();
        save_connection(&cfg, &f, &store).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
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
            mongostudio_core::GSSAPI_AVAILABLE
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
}
