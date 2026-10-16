//! Connection-string parsing and building. Pure and synchronous. Errors are fixed strings and
//! never contain any part of the input (it can hold a password).
use crate::connection::{AuthMechanism, ConnectionConfig};
use crate::error::{CoreError, Result};
use crate::secrets::SecretString;

pub const MASK: &str = "****";

const SECRET_QUERY_KEYS: [&str; 3] = [
    "tlscertificatekeyfilepassword",
    "sslclientcertificatekeypassword",
    "proxypassword",
];

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedUri {
    /// The connection string without userinfo and without the options held as form fields.
    pub uri: String,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub auth_source: Option<String>,
    pub auth: Option<AuthMechanism>,
    pub tls: Option<bool>,
}

#[derive(Clone, Copy)]
pub enum PasswordMode<'a> {
    Omit,
    Masked,
    Include(&'a str),
}

/// Never prints the password carried by `Include`.
impl std::fmt::Debug for PasswordMode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Omit => f.write_str("Omit"),
            Self::Masked => f.write_str("Masked"),
            Self::Include(_) => f.write_str("Include(<redacted>)"),
        }
    }
}

fn invalid(msg: &'static str) -> CoreError {
    CoreError::InvalidUri(msg.into())
}

/// A connection string cut into its scheme, userinfo, hosts and the rest (path and query).
struct Split<'a> {
    srv: bool,
    userinfo: Option<&'a str>,
    hosts: &'a str,
    tail: &'a str,
}

/// Splits a trimmed connection string. Userinfo is everything before the LAST '@' of the
/// authority (which ends at the first '/' or '?'). Errors never carry the input.
fn split_uri(input: &str) -> Result<Split<'_>> {
    let (srv, rest) = if let Some(r) = input.strip_prefix("mongodb+srv://") {
        (true, r)
    } else if let Some(r) = input.strip_prefix("mongodb://") {
        (false, r)
    } else {
        return Err(invalid("scheme must be mongodb:// or mongodb+srv://"));
    };
    if rest.contains('#') {
        return Err(malformed());
    }
    let (authority, tail) = match rest.find(['/', '?']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let (userinfo, hosts) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    if hosts.is_empty() {
        return Err(invalid("missing host"));
    }
    validate_hosts(hosts, srv)?;
    Ok(Split {
        srv,
        userinfo,
        hosts,
        tail,
    })
}

/// Strictly checks the scheme and the host list of a connection string, exactly as `parse_uri`
/// does, without reading credentials or options. Errors never carry the input.
pub fn check_hosts(uri: &str) -> Result<()> {
    split_uri(uri.trim()).map(|_| ())
}

pub fn parse_uri(input: &str) -> Result<ParsedUri> {
    let input = input.trim();
    let Split {
        srv,
        userinfo,
        hosts,
        tail,
    } = split_uri(input)?;
    let scheme = if srv { "mongodb+srv://" } else { "mongodb://" };
    let (username, password) = parse_userinfo(userinfo)?;
    let (path, query) = match tail.split_once('?') {
        Some((p, q)) => (p, q),
        None => (tail, ""),
    };
    if path.contains('@') || query.contains('@') {
        return Err(malformed());
    }
    let mut kept = Vec::new();
    let (mut auth_source, mut auth, mut tls) = (None, None, None);
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').ok_or_else(malformed)?;
        let lower = key.to_ascii_lowercase();
        if SECRET_QUERY_KEYS.contains(&lower.as_str()) {
            return Err(invalid("secrets must not be embedded in the uri"));
        }
        match lower.as_str() {
            "authsource" => auth_source = Some(percent_decode(value)?),
            "authmechanism" => auth = Some(parse_mechanism(value)?),
            "tls" | "ssl" => tls = Some(parse_bool(value)?),
            _ => kept.push(pair),
        }
    }
    if auth.is_none() && username.is_some() {
        auth = Some(AuthMechanism::ScramSha256);
    }
    let mut uri = format!("{scheme}{hosts}{path}");
    if !kept.is_empty() {
        if path.is_empty() {
            uri.push('/');
        }
        uri.push('?');
        uri.push_str(&kept.join("&"));
    }
    Ok(ParsedUri {
        uri,
        username,
        password,
        auth_source: auth_source.filter(|s| !s.is_empty()),
        auth,
        tls,
    })
}

fn malformed() -> CoreError {
    invalid("malformed connection string")
}

fn parse_bool(value: &str) -> Result<bool> {
    if value.eq_ignore_ascii_case("true") {
        Ok(true)
    } else if value.eq_ignore_ascii_case("false") {
        Ok(false)
    } else {
        Err(malformed())
    }
}

type Credentials = (Option<String>, Option<SecretString>);

/// Splits `user[:password]`. A second raw `@`, a password without a user, an empty password and
/// a password equal to the display mask are all handled here; none of them echo the input.
fn parse_userinfo(userinfo: Option<&str>) -> Result<Credentials> {
    let Some(u) = userinfo else {
        return Ok((None, None));
    };
    if u.contains('@') {
        return Err(malformed());
    }
    let (user, pass) = match u.split_once(':') {
        Some((user, pass)) => (user, Some(pass)),
        None => (u, None),
    };
    let user = percent_decode(user)?;
    let pass = pass.map(percent_decode).transpose()?;
    let pass = pass.filter(|p| !p.is_empty() && p != MASK);
    if user.is_empty() && pass.is_some() {
        return Err(malformed());
    }
    Ok((
        (!user.is_empty()).then_some(user),
        pass.map(SecretString::new),
    ))
}

fn validate_hosts(hosts: &str, srv: bool) -> Result<()> {
    let entries: Vec<&str> = hosts.split(',').collect();
    if srv && entries.len() != 1 {
        return Err(malformed());
    }
    entries.iter().try_for_each(|e| validate_host(e, srv))
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

fn validate_host(entry: &str, srv: bool) -> Result<()> {
    if entry.starts_with("%2F") || entry.starts_with("%2f") {
        let ok = !srv && entry.ends_with(".sock") && is_encoded_path(entry);
        return if ok { Ok(()) } else { Err(malformed()) };
    }
    let (host_ok, port) = if let Some(inner) = entry.strip_prefix('[') {
        let (addr, after) = inner.split_once(']').ok_or_else(malformed)?;
        let addr_ok = !addr.is_empty()
            && addr
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or_else(malformed)?),
        };
        (addr_ok, port)
    } else {
        let (host, port) = match entry.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (entry, None),
        };
        (!host.is_empty() && host.bytes().all(is_name_byte), port)
    };
    if !host_ok {
        return Err(malformed());
    }
    match port {
        None => Ok(()),
        Some(_) if srv => Err(malformed()),
        Some(p) => {
            let digits = (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit());
            match p.parse::<u16>() {
                Ok(n) if digits && n > 0 => Ok(()),
                _ => Err(malformed()),
            }
        }
    }
}

fn is_encoded_path(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let ok = b
                .get(i + 1..i + 3)
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit));
            if !ok {
                return false;
            }
            i += 3;
        } else if is_name_byte(b[i]) {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

fn parse_mechanism(value: &str) -> Result<AuthMechanism> {
    Ok(match value.to_ascii_uppercase().as_str() {
        "SCRAM-SHA-1" => AuthMechanism::ScramSha1,
        "SCRAM-SHA-256" => AuthMechanism::ScramSha256,
        "MONGODB-X509" => AuthMechanism::X509,
        "GSSAPI" => AuthMechanism::Gssapi,
        "PLAIN" => AuthMechanism::Plain,
        _ => return Err(invalid("unsupported authentication mechanism")),
    })
}

fn mechanism_name(auth: AuthMechanism) -> Option<&'static str> {
    match auth {
        AuthMechanism::None => None,
        AuthMechanism::ScramSha1 => Some("SCRAM-SHA-1"),
        AuthMechanism::ScramSha256 => Some("SCRAM-SHA-256"),
        AuthMechanism::X509 => Some("MONGODB-X509"),
        AuthMechanism::Gssapi => Some("GSSAPI"),
        AuthMechanism::Plain => Some("PLAIN"),
    }
}

/// Builds a connection string from a saved config. `Masked` shows `****` in place of the
/// password; only `Include` ever carries a real one. Without a username there is no userinfo.
pub fn build_uri(cfg: &ConnectionConfig, password: PasswordMode<'_>) -> String {
    let (scheme, rest) = match cfg.uri.strip_prefix("mongodb+srv://") {
        Some(r) => ("mongodb+srv://", r),
        None => (
            "mongodb://",
            cfg.uri.strip_prefix("mongodb://").unwrap_or(&cfg.uri),
        ),
    };
    // A stored uri may already carry `user@`; drop it so we never emit `bob@user@host`.
    let rest = if cfg.username.is_some() {
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        match rest[..end].rfind('@') {
            Some(at) => &rest[at + 1..],
            None => rest,
        }
    } else {
        rest
    };
    let userinfo = cfg.username.as_deref().map(|user| {
        let user = percent_encode(user);
        match password {
            PasswordMode::Omit => format!("{user}@"),
            PasswordMode::Masked => format!("{user}:{MASK}@"),
            PasswordMode::Include(pw) => format!("{user}:{}@", percent_encode(pw)),
        }
    });
    let mut extras = Vec::new();
    if let Some(source) = &cfg.auth_source {
        extras.push(format!("authSource={}", percent_encode(source)));
    }
    if let Some(name) = mechanism_name(cfg.auth) {
        extras.push(format!("authMechanism={name}"));
    }
    if cfg.tls.enabled {
        extras.push("tls=true".to_owned());
    }
    let (before_query, query) = match rest.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (rest, None),
    };
    let mut out = format!("{scheme}{}{before_query}", userinfo.unwrap_or_default());
    let has_path = before_query.contains('/');
    let query = query.filter(|q| !q.is_empty());
    if query.is_some() || !extras.is_empty() {
        if !has_path {
            out.push('/');
        }
        out.push('?');
        let mut parts: Vec<&str> = query.into_iter().collect();
        parts.extend(extras.iter().map(String::as_str));
        out.push_str(&parts.join("&"));
    }
    out
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| invalid("malformed connection string"))?;
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| invalid("malformed connection string"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn password_mode_debug_never_prints_the_password() {
        let shown = format!("{:?}", PasswordMode::Include("hunter2"));
        assert_eq!(shown, "Include(<redacted>)");
        assert!(!format!("{:?}", [PasswordMode::Omit, PasswordMode::Masked]).contains("redacted"));
    }

    use super::*;
    use crate::connection::ConnectionConfig;

    #[test]
    fn parses_credentials_options_and_strips_them_from_the_uri() {
        let p = parse_uri(
            "mongodb://bob:s3cr%40t@h1:27017,h2:27018/app?replicaSet=rs0&authSource=admin&authMechanism=SCRAM-SHA-256&tls=true",
        )
        .unwrap();
        assert_eq!(p.uri, "mongodb://h1:27017,h2:27018/app?replicaSet=rs0");
        assert_eq!(p.username.as_deref(), Some("bob"));
        assert_eq!(
            p.password.as_ref().map(SecretString::expose),
            Some("s3cr@t")
        );
        assert_eq!(p.auth_source.as_deref(), Some("admin"));
        assert_eq!(p.auth, Some(AuthMechanism::ScramSha256));
        assert_eq!(p.tls, Some(true));
    }

    #[test]
    fn username_without_a_mechanism_defaults_to_scram_sha_256() {
        let p = parse_uri("mongodb://bob:pw@h/").unwrap();
        assert_eq!(p.auth, Some(AuthMechanism::ScramSha256));
    }

    #[test]
    fn no_userinfo_means_no_credentials_and_no_auth_change() {
        let p = parse_uri("mongodb+srv://cluster0.example.com/db").unwrap();
        assert_eq!((p.username, p.password, p.auth), (None, None, None));
        assert_eq!(p.uri, "mongodb+srv://cluster0.example.com/db");
    }

    #[test]
    fn accepts_a_username_without_a_password() {
        let p = parse_uri("mongodb://bob@h/db").unwrap();
        assert_eq!(p.username.as_deref(), Some("bob"));
        assert_eq!(p.password, None);
    }

    #[test]
    fn rejects_bad_input_without_echoing_it() {
        let secret = "hunter2-do-not-leak";
        for bad in [
            format!("http://u:{secret}@h"),
            format!("mongodb://u:{secret}@"),
            format!("mongodb://u:%zz{secret}@h"),
            format!("mongodb://u:{secret}@h/?authMechanism=NOPE"),
            format!("mongodb://u:p@h/?tlsCertificateKeyFilePassword={secret}"),
            format!("mongodb://u:p@h/?sslClientCertificateKeyPassword={secret}"),
            String::new(),
        ] {
            let err = parse_uri(&bad).unwrap_err().to_string();
            assert!(!err.contains(secret), "{bad:?} leaked into {err:?}");
        }
    }

    #[test]
    fn password_round_trips_for_awkward_characters() {
        for pw in ["p@ss:/%word", "a b?c#d", "ünï€", "100%", "::@@//"] {
            let mut cfg = ConnectionConfig::new("a", "mongodb://h:27017/db");
            cfg.username = Some("us:er@x".into());
            let built = build_uri(&cfg, PasswordMode::Include(pw));
            let parsed = parse_uri(&built).unwrap();
            assert_eq!(parsed.username.as_deref(), Some("us:er@x"));
            assert_eq!(
                parsed.password.as_ref().map(SecretString::expose),
                Some(pw),
                "{built}"
            );
            assert_eq!(parsed.uri, "mongodb://h:27017/db");
        }
    }

    #[test]
    fn build_masks_by_default_and_omits_when_asked() {
        let mut cfg = ConnectionConfig::new("a", "mongodb://h/db?replicaSet=rs0");
        cfg.username = Some("bob".into());
        cfg.auth_source = Some("admin".into());
        assert_eq!(
            build_uri(&cfg, PasswordMode::Masked),
            "mongodb://bob:****@h/db?replicaSet=rs0&authSource=admin"
        );
        assert_eq!(
            build_uri(&cfg, PasswordMode::Omit),
            "mongodb://bob@h/db?replicaSet=rs0&authSource=admin"
        );
    }

    #[test]
    fn build_without_a_username_has_no_userinfo_even_if_a_password_is_given() {
        let cfg = ConnectionConfig::new("a", "mongodb://h");
        assert_eq!(build_uri(&cfg, PasswordMode::Include("pw")), "mongodb://h");
    }

    #[test]
    fn build_adds_the_mechanism_and_tls_options() {
        let mut cfg = ConnectionConfig::new("a", "mongodb://h/db");
        cfg.username = Some("bob".into());
        cfg.auth = AuthMechanism::ScramSha1;
        cfg.tls.enabled = true;
        let built = build_uri(&cfg, PasswordMode::Omit);
        assert!(built.contains("authMechanism=SCRAM-SHA-1"));
        assert!(built.contains("tls=true"));
    }

    #[test]
    fn no_path_with_kept_options_gains_a_slash() {
        let p = parse_uri("mongodb://h?x=1&tls=true").unwrap();
        assert_eq!(p.uri, "mongodb://h/?x=1");
        let p = parse_uri("mongodb://h/?authSource=admin").unwrap();
        assert_eq!(p.uri, "mongodb://h/");
        assert_eq!(p.auth_source.as_deref(), Some("admin"));
    }

    #[test]
    fn empty_auth_source_is_dropped_and_keys_are_case_insensitive() {
        let p = parse_uri("mongodb://h/db?authSource=&TLS=TRUE&AuthMechanism=plain").unwrap();
        assert_eq!(p.auth_source, None);
        assert_eq!(p.tls, Some(true));
        assert_eq!(p.auth, Some(AuthMechanism::Plain));
        assert_eq!(p.uri, "mongodb://h/db");
    }

    #[test]
    fn duplicate_keys_keep_the_last_value_and_repeated_kept_keys_stay() {
        let p =
            parse_uri("mongodb://h/db?authSource=a&authSource=b&readPreference=x&readPreference=y")
                .unwrap();
        assert_eq!(p.auth_source.as_deref(), Some("b"));
        assert_eq!(p.uri, "mongodb://h/db?readPreference=x&readPreference=y");
    }

    #[test]
    fn a_second_raw_at_sign_in_the_userinfo_is_rejected() {
        assert!(parse_uri("mongodb://a:b@c@h/db").is_err());
        assert!(parse_uri("mongodb://bob:p@ss@h/db").is_err());
        let p = parse_uri("mongodb://bob:p%40ss@h/db").unwrap();
        assert_eq!(p.password.as_ref().map(SecretString::expose), Some("p@ss"));
    }

    const FRAG: &str = "SeCr3tFrag";

    fn assert_no_fragment(input: &str) {
        match parse_uri(input) {
            Err(e) => assert!(!e.to_string().contains(FRAG), "{input}"),
            Ok(p) => {
                assert!(!p.uri.contains(FRAG), "uri leaked for {input}: {}", p.uri);
                assert!(!p.username.unwrap_or_default().contains(FRAG), "{input}");
                assert!(!p.auth_source.unwrap_or_default().contains(FRAG), "{input}");
            }
        }
    }

    #[test]
    fn password_fragments_never_land_in_the_uri() {
        let bad = [
            format!("mongodb://bob:pa/{FRAG}@h/db"),
            format!("mongodb://bob:pa?{FRAG}@h/db"),
            format!("mongodb://bob:{FRAG}"),
            format!("mongodb://bob:{FRAG}@h:27017@evil/db"),
            format!("mongodb://bob:pa#{FRAG}@h/db"),
            format!("mongodb://bob:pa/{FRAG}@h/db?authSource={FRAG}"),
            format!("mongodb://bob:{FRAG}@h/db?x@{FRAG}=1"),
        ];
        for input in &bad {
            assert_no_fragment(input);
            assert!(parse_uri(input).is_err(), "{input} should be rejected");
        }
    }

    #[test]
    fn property_no_ok_result_contains_the_password_text() {
        let inputs = [
            "mongodb://bob:SeCr3tFrag/x@h/db",
            "mongodb://bob:SeCr3tFrag?x@h/db",
            "mongodb://bob:SeCr3tFrag",
            "mongodb://bob:SeCr3tFrag@h:1@h2",
            "mongodb://bob:Se/Cr3t@h",
            "mongodb://bob:a,b@h",
            "mongodb://bob:SeCr3tFrag@@h",
            "mongodb://bob:SeCr3tFrag@h/db@x",
            "mongodb://bob:SeCr3tFrag@h/db?a@b=1",
            "mongodb://bob:SeCr3tFrag@h/db?a",
        ];
        for input in inputs {
            if let Ok(p) = parse_uri(input) {
                let pw = input.split_once(':').and_then(|(_, r)| r.split_once(':'));
                let pw = pw.map(|(_, r)| r.split('@').next().unwrap_or(""));
                if let Some(pw) = pw.filter(|s| !s.is_empty()) {
                    assert!(!p.uri.contains(pw), "{input} -> {}", p.uri);
                }
            }
        }
    }

    #[test]
    fn check_hosts_accepts_valid_hosts_with_or_without_userinfo() {
        for ok in [
            "mongodb://h",
            "mongodb://u@h",
            "mongodb://h1:27017,h2:27018",
            "mongodb://[::1]:27017",
            "mongodb+srv://cluster.example.com",
            "mongodb+srv://u@cluster.example.com/db?retryWrites=true",
            "mongodb://%2Ftmp%2Fmongodb-27017.sock",
        ] {
            assert!(check_hosts(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn check_hosts_rejects_a_password_that_leaks_into_the_host_without_echo() {
        for bad in [
            "mongodb://u:pa/ss@h",
            "mongodb://u@",
            "mongodb://",
            "mongodb://h:0",
            "mongodb+srv://h1,h2",
            "http://h",
        ] {
            let err = check_hosts(bad).unwrap_err();
            assert!(matches!(err, CoreError::InvalidUri(_)), "{bad}");
            assert!(!err.to_string().contains("pa"), "{bad}: {err}");
        }
    }

    #[test]
    fn hosts_are_validated() {
        for ok in [
            "mongodb://h",
            "mongodb://h.example-1.com:27017",
            "mongodb://h1:1,h2:65535,h3",
            "mongodb://[::1]:27017",
            "mongodb://[fe80::1]",
            "mongodb://%2Ftmp%2Fmongodb-27017.sock",
            "mongodb+srv://cluster0.example.com/db",
        ] {
            assert!(parse_uri(ok).is_ok(), "{ok}");
        }
        for bad in [
            "mongodb://h:",
            "mongodb://h:0",
            "mongodb://h:65536",
            "mongodb://h:123456",
            "mongodb://h:12a",
            "mongodb://h,,h2",
            "mongodb://h,",
            "mongodb://h h2",
            "mongodb://[::1",
            "mongodb://[zz]:1",
            "mongodb://%2Ftmp%2Fx",
            "mongodb://h%2F",
            "mongodb+srv://a.example.com,b.example.com",
            "mongodb+srv://a.example.com:27017",
        ] {
            let err = parse_uri(bad).unwrap_err().to_string();
            assert!(err.contains("malformed"), "{bad}: {err}");
        }
    }

    #[test]
    fn raw_at_in_the_path_or_query_and_valueless_pairs_are_rejected() {
        for bad in [
            "mongodb://h/db@x",
            "mongodb://h/db?a=b@c",
            "mongodb://h/db?a",
            "mongodb://h/db?a=1&b",
        ] {
            assert!(parse_uri(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn tls_accepts_only_true_or_false() {
        assert_eq!(
            parse_uri("mongodb://h/?tls=FALSE").unwrap().tls,
            Some(false)
        );
        assert_eq!(parse_uri("mongodb://h/?ssl=True").unwrap().tls, Some(true));
        for bad in ["tls=1", "tls=yes", "tls=", "ssl=on"] {
            assert!(parse_uri(&format!("mongodb://h/?{bad}")).is_err(), "{bad}");
        }
    }

    #[test]
    fn empty_and_masked_passwords_are_no_password_and_a_password_needs_a_user() {
        assert_eq!(parse_uri("mongodb://bob:@h").unwrap().password, None);
        let p = parse_uri("mongodb://bob:****@h").unwrap();
        assert_eq!(p.password, None);
        assert_eq!(p.username.as_deref(), Some("bob"));
        let err = parse_uri("mongodb://:SeCr3tFrag@h")
            .unwrap_err()
            .to_string();
        assert!(!err.contains(FRAG));
        assert!(parse_uri("mongodb://@h").is_ok());
    }

    #[test]
    fn build_replaces_userinfo_already_in_the_stored_uri() {
        let mut cfg = ConnectionConfig::new("a", "mongodb://old@h1,h2/db?x=1");
        cfg.username = Some("bob".into());
        assert_eq!(
            build_uri(&cfg, PasswordMode::Omit),
            "mongodb://bob@h1,h2/db?x=1"
        );
        let mut cfg = ConnectionConfig::new("a", "mongodb+srv://old@h/db");
        cfg.username = Some("bob".into());
        assert_eq!(
            build_uri(&cfg, PasswordMode::Masked),
            "mongodb+srv://bob:****@h/db"
        );
    }

    #[test]
    fn empty_host_with_credentials_is_rejected_without_echo() {
        for bad in [
            "mongodb://u:pw-leak@/db",
            "mongodb://u:pw-leak@?x=1",
            "mongodb://:pw-leak@",
        ] {
            let err = parse_uri(bad).unwrap_err().to_string();
            assert!(!err.contains("pw-leak"), "{err}");
        }
    }

    #[test]
    fn percent_decode_never_panics_on_odd_input() {
        for s in [
            "%", "%4", "%zz", "%4g", "%%%", "%C3", "%C3%28", "\u{e9}%", "a%é", "%é9",
        ] {
            let _ = percent_decode(s);
        }
        assert!(percent_decode("%").is_err());
        assert!(percent_decode("%4").is_err());
        assert!(percent_decode("%zz").is_err());
        assert!(percent_decode("%C3").is_err());
        assert_eq!(percent_decode("%C3%A9").unwrap(), "\u{e9}");
        let long = "%41".repeat(100_000);
        assert_eq!(percent_decode(&long).unwrap().len(), 100_000);
        let long_uri = format!("mongodb://u:{}@h", "a".repeat(200_000));
        assert!(parse_uri(&long_uri).is_ok());
    }

    #[test]
    fn raw_slash_in_a_password_never_leaks_into_the_error() {
        match parse_uri("mongodb://u:pa/ss-leak@h/db") {
            Ok(p) => assert_eq!(p.password, None),
            Err(e) => assert!(!e.to_string().contains("ss-leak")),
        }
    }

    #[test]
    fn build_handles_queries_without_a_path() {
        let mut cfg = ConnectionConfig::new("a", "mongodb://h?x=1");
        cfg.username = Some("bob".into());
        cfg.tls.enabled = true;
        assert_eq!(
            build_uri(&cfg, PasswordMode::Omit),
            "mongodb://bob@h/?x=1&tls=true"
        );
        let cfg2 = ConnectionConfig::new("a", "mongodb://h?x=1");
        assert_eq!(build_uri(&cfg2, PasswordMode::Omit), "mongodb://h/?x=1");
        let mut cfg3 = ConnectionConfig::new("a", "mongodb://h/");
        cfg3.tls.enabled = true;
        assert_eq!(
            build_uri(&cfg3, PasswordMode::Omit),
            "mongodb://h/?tls=true"
        );
    }
}
