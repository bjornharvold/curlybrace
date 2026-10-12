use crate::connection::{AuthMechanism, ConnectionConfig};
use crate::error::{CoreError, Result};
use bson::doc;
use mongodb::Client;
use mongodb::options::{
    AuthMechanism as DriverMechanism, ClientOptions, ServerAddress, Tls, TlsOptions,
};
use std::net::SocketAddr;
use std::time::Duration;

pub const SERVER_SELECTION_TIMEOUT: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Query-string parameters that carry secrets and must never live in the stored URI.
const SECRET_QUERY_PARAMS: [&str; 2] = ["tlscertificatekeyfilepassword", "proxypassword"];

pub async fn build_client_options(
    cfg: &ConnectionConfig,
    password: Option<&str>,
    tunnel: Option<SocketAddr>,
) -> Result<ClientOptions> {
    cfg.validate()?;
    // Never echo the driver's parse error: it can quote parts of the URI.
    let mut options = ClientOptions::parse(&cfg.uri)
        .await
        .map_err(|_| CoreError::InvalidUri("the connection uri could not be parsed".into()))?;
    reject_embedded_secrets(&cfg.uri, &options)?;
    options.server_selection_timeout = Some(SERVER_SELECTION_TIMEOUT);
    options.connect_timeout = Some(CONNECT_TIMEOUT);
    apply_credential(&mut options, cfg, password)?;
    apply_tls(&mut options, cfg);
    if let Some(addr) = tunnel {
        options.hosts = vec![ServerAddress::Tcp {
            host: addr.ip().to_string(),
            port: Some(addr.port()),
        }];
        options.direct_connection = Some(true);
    }
    Ok(options)
}

pub async fn connect(
    cfg: &ConnectionConfig,
    password: Option<&str>,
    tunnel: Option<SocketAddr>,
) -> Result<Client> {
    let options = build_client_options(cfg, password, tunnel).await?;
    let client = Client::with_options(options).map_err(|e| CoreError::Database(e.to_string()))?;
    client
        .database("admin")
        .run_command(doc! { "ping": 1 })
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
    Ok(client)
}

fn reject_embedded_secrets(uri: &str, options: &ClientOptions) -> Result<()> {
    let has_password = options
        .credential
        .as_ref()
        .is_some_and(|c| c.password.is_some());
    let query = uri.split_once('?').map_or("", |(_, q)| q).to_lowercase();
    let has_query_secret = query
        .split('&')
        .filter_map(|kv| kv.split('=').next())
        .any(|k| SECRET_QUERY_PARAMS.contains(&k));
    if has_password || has_query_secret {
        return Err(CoreError::InvalidUri(
            "secrets must not be embedded in the uri".into(),
        ));
    }
    Ok(())
}

fn driver_mechanism(auth: AuthMechanism) -> Result<Option<DriverMechanism>> {
    Ok(Some(match auth {
        AuthMechanism::None => return Ok(None),
        AuthMechanism::ScramSha1 => DriverMechanism::ScramSha1,
        AuthMechanism::ScramSha256 => DriverMechanism::ScramSha256,
        AuthMechanism::X509 => DriverMechanism::MongoDbX509,
        AuthMechanism::Plain => DriverMechanism::Plain,
        #[cfg(feature = "gssapi")]
        AuthMechanism::Gssapi => DriverMechanism::Gssapi,
        #[cfg(not(feature = "gssapi"))]
        AuthMechanism::Gssapi => {
            return Err(CoreError::Unsupported(
                "gssapi requires a build with the gssapi feature".into(),
            ));
        }
    }))
}

fn apply_credential(
    options: &mut ClientOptions,
    cfg: &ConnectionConfig,
    password: Option<&str>,
) -> Result<()> {
    let Some(mechanism) = driver_mechanism(cfg.auth)? else {
        return Ok(());
    };
    let external = matches!(
        cfg.auth,
        AuthMechanism::X509 | AuthMechanism::Plain | AuthMechanism::Gssapi
    );
    let source = match (&cfg.auth_source, external) {
        (Some(s), _) => Some(s.clone()),
        (None, true) => Some("$external".to_string()),
        (None, false) => None,
    };
    let mut credential = options.credential.take().unwrap_or_default();
    credential.mechanism = Some(mechanism);
    credential.username = cfg.username.clone().or(credential.username);
    credential.password = password.map(str::to_owned);
    credential.source = source.or(credential.source);
    options.credential = Some(credential);
    Ok(())
}

fn apply_tls(options: &mut ClientOptions, cfg: &ConnectionConfig) {
    if !cfg.tls.enabled {
        return;
    }
    let tls = TlsOptions::builder()
        .ca_file_path(cfg.tls.ca_file.clone())
        .cert_key_file_path(cfg.tls.client_cert_key_file.clone())
        .allow_invalid_certificates(cfg.tls.allow_invalid_certs.then_some(true))
        .build();
    options.tls = Some(Tls::Enabled(tls));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn password_with_special_chars_is_set_on_credential_not_in_uri() {
        let mut c = ConnectionConfig::new("a", "mongodb://localhost:27017/admin");
        c.auth = AuthMechanism::ScramSha256;
        c.username = Some("bob".into());
        let o = build_client_options(&c, Some("p@ss:/%w ord"), None)
            .await
            .unwrap();
        let cred = o.credential.unwrap();
        assert_eq!(cred.username.as_deref(), Some("bob"));
        assert_eq!(cred.password.as_deref(), Some("p@ss:/%w ord"));
        assert_eq!(
            cred.mechanism,
            Some(mongodb::options::AuthMechanism::ScramSha256)
        );
    }

    #[tokio::test]
    async fn timeout_is_bounded() {
        let o = build_client_options(
            &ConnectionConfig::new("a", "mongodb://localhost"),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(o.server_selection_timeout, Some(SERVER_SELECTION_TIMEOUT));
        assert!(o.connect_timeout.is_some());
    }

    #[tokio::test]
    async fn tunnel_overrides_hosts_and_forces_direct() {
        let addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let o = build_client_options(
            &ConnectionConfig::new("a", "mongodb://db.internal:27017"),
            None,
            Some(addr),
        )
        .await
        .unwrap();
        assert_eq!(o.hosts.len(), 1);
        assert_eq!(o.hosts[0].to_string(), "127.0.0.1:50000");
        assert_eq!(o.direct_connection, Some(true));
    }

    #[tokio::test]
    async fn x509_and_plain_map_to_driver_mechanisms() {
        let mut x = ConnectionConfig::new("a", "mongodb://h");
        x.auth = AuthMechanism::X509;
        x.tls.enabled = true;
        x.tls.client_cert_key_file = Some("/tmp/c.pem".into());
        let cred = build_client_options(&x, None, None)
            .await
            .unwrap()
            .credential
            .unwrap();
        assert_eq!(
            cred.mechanism,
            Some(mongodb::options::AuthMechanism::MongoDbX509)
        );
        assert_eq!(cred.source.as_deref(), Some("$external"));

        let mut p = ConnectionConfig::new("a", "mongodb://h");
        p.auth = AuthMechanism::Plain;
        p.username = Some("ldapuser".into());
        let cred = build_client_options(&p, Some("pw"), None)
            .await
            .unwrap()
            .credential
            .unwrap();
        assert_eq!(cred.mechanism, Some(mongodb::options::AuthMechanism::Plain));
        assert_eq!(cred.source.as_deref(), Some("$external"));

        let none = ConnectionConfig::new("a", "mongodb://h");
        assert!(
            build_client_options(&none, None, None)
                .await
                .unwrap()
                .credential
                .is_none()
        );
    }

    #[cfg(not(feature = "gssapi"))]
    #[tokio::test]
    async fn gssapi_without_feature_is_unsupported() {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.auth = AuthMechanism::Gssapi;
        assert!(matches!(
            build_client_options(&c, None, None).await,
            Err(CoreError::Unsupported(_))
        ));
    }

    #[tokio::test]
    async fn unreachable_host_fails_within_bound() {
        let c = ConnectionConfig::new("a", "mongodb://127.0.0.1:1");
        let start = std::time::Instant::now();
        let err = connect(&c, None, None).await.unwrap_err();
        assert!(matches!(err, CoreError::Database(_)));
        assert!(start.elapsed() < SERVER_SELECTION_TIMEOUT + Duration::from_secs(3));
    }

    #[tokio::test]
    async fn tls_options_are_applied() {
        let mut c = ConnectionConfig::new("a", "mongodb://h");
        c.tls.enabled = true;
        c.tls.ca_file = Some("/tmp/ca.pem".into());
        c.tls.allow_invalid_certs = true;
        let o = build_client_options(&c, None, None).await.unwrap();
        match o.tls {
            Some(mongodb::options::Tls::Enabled(t)) => {
                assert_eq!(
                    t.ca_file_path.as_deref(),
                    Some(std::path::Path::new("/tmp/ca.pem"))
                );
                assert_eq!(t.allow_invalid_certificates, Some(true));
            }
            other => panic!("unexpected tls: {other:?}"),
        }
    }

    async fn assert_secret_rejected(uri: &str, secret: &str) {
        let err = build_client_options(&ConnectionConfig::new("a", uri), None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::InvalidUri(_)), "{uri}: {err:?}");
        assert!(!err.to_string().contains(secret), "{uri}: leaked secret");
        assert!(!format!("{err:?}").contains(secret), "{uri}: leaked secret");
    }

    #[tokio::test]
    async fn password_with_unescaped_slash_is_rejected_without_echo() {
        assert_secret_rejected("mongodb://bob:pa/ss@h", "pa/ss").await;
    }

    #[tokio::test]
    async fn query_string_secrets_are_rejected_without_echo() {
        assert_secret_rejected(
            "mongodb://h/db?tlsCertificateKeyFilePassword=hunter2",
            "hunter2",
        )
        .await;
        assert_secret_rejected(
            "mongodb://h/?TLSCERTIFICATEKEYFILEPASSWORD=hunter2",
            "hunter2",
        )
        .await;
        assert_secret_rejected("mongodb://h/?proxyPassword=hunter2&proxyHost=p", "hunter2").await;
    }
}
