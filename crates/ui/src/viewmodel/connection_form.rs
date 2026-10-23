//! Editable connection form: validation, config mapping and secret persistence.
use curlybrace_core::GSSAPI_AVAILABLE;
use curlybrace_core::connection::{
    AuthMechanism, ConnectionConfig, SavedColor, SshAuth, SshConfig, TlsConfig,
};
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
    /// The colour as stored. A value this build does not recognise is kept as it is: the picker
    /// shows it as "No colour", and it is written back untouched until the user picks a preset
    /// or clicks "No colour".
    pub color: Option<SavedColor>,
    pub production: bool,
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
            color: None,
            production: false,
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
            color: self.color.clone(),
            production: self.production,
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
            color: c.color.clone(),
            production: c.production,
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
#[path = "connection_form_tests.rs"]
mod tests;
