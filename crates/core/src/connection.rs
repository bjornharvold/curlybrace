use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthMechanism { None, ScramSha1, ScramSha256, X509, Gssapi, Plain }

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub ca_file: Option<PathBuf>,
    pub client_cert_key_file: Option<PathBuf>,
    pub allow_invalid_certs: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SshAuth { Password, PrivateKey { path: PathBuf } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SshConfig { pub host: String, pub port: u16, pub username: String, pub auth: SshAuth }

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
}

impl ConnectionConfig {
    pub fn new(name: &str, uri: &str) -> Self {
        Self {
            id: Uuid::new_v4(), name: name.into(), uri: uri.into(),
            auth: AuthMechanism::None, username: None, auth_source: None,
            tls: TlsConfig::default(), ssh: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(CoreError::InvalidUri("blank connection name".into()));
        }
        self.validate_uri()?;
        if self.auth == AuthMechanism::X509
            && !(self.tls.enabled && self.tls.client_cert_key_file.is_some())
        {
            return Err(CoreError::InvalidUri("x509 requires tls and a client certificate".into()));
        }
        if let Some(ssh) = &self.ssh {
            if ssh.host.trim().is_empty() || ssh.username.trim().is_empty() || ssh.port == 0 {
                return Err(CoreError::Ssh("host, username and a non-zero port are required".into()));
            }
        }
        Ok(())
    }

    fn validate_uri(&self) -> Result<()> {
        let rest = self.uri.strip_prefix("mongodb+srv://")
            .or_else(|| self.uri.strip_prefix("mongodb://"))
            .ok_or_else(|| CoreError::InvalidUri("scheme must be mongodb:// or mongodb+srv://".into()))?;
        let authority = rest.split(['/', '?']).next().unwrap_or("");
        if authority.is_empty() {
            return Err(CoreError::InvalidUri("missing host".into()));
        }
        // userinfo is everything before the last '@' in the authority
        if let Some((userinfo, _)) = authority.rsplit_once('@') {
            if userinfo.contains(':') {
                return Err(CoreError::InvalidUri("password must not be embedded in the uri".into()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_and_srv_uris() {
        assert!(ConnectionConfig::new("a", "mongodb://localhost:27017").validate().is_ok());
        assert!(ConnectionConfig::new("a", "mongodb+srv://cluster0.example.com/db").validate().is_ok());
    }

    #[test]
    fn rejects_wrong_scheme_and_empty() {
        for bad in ["", "http://x", "localhost:27017"] {
            assert!(matches!(
                ConnectionConfig::new("a", bad).validate(),
                Err(CoreError::InvalidUri(_))
            ), "{bad}");
        }
    }

    #[test]
    fn rejects_embedded_password() {
        let err = ConnectionConfig::new("a", "mongodb://bob:secret@h/db").validate().unwrap_err();
        assert!(matches!(err, CoreError::InvalidUri(_)));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn rejects_blank_name_and_bad_ssh_port() {
        assert!(ConnectionConfig::new("  ", "mongodb://h").validate().is_err());
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.ssh = Some(SshConfig { host: "b".into(), port: 0, username: "u".into(), auth: SshAuth::Password });
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
}
