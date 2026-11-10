//! The `mongosh` command line and stdin payload for the Shell tab.
//!
//! The password never reaches argv or the environment: the connect line (with the credentials in
//! the URI) is the first thing written to the child's stdin, and the user's script follows it.
use crate::client::{effective_auth_source, tls_blocks_tunnel, uri_has_secret_query_param};
use crate::compare::export::sanitize::js_string;
use crate::connection::{AuthMechanism, ConnectionConfig};
use crate::error::CoreError;
use crate::uri_problem::UriProblem;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::Path;

const SRV_SCHEME: &str = "mongodb+srv://";
const PLAIN_SCHEME: &str = "mongodb://";

/// The prompt the session uses from the preamble on. Fixed, so output never carries the default
/// prompt (`rs0 [primary] admin> `, `[mongos] admin> `, ...) and the classifier strips exactly
/// this token. U+0001 does not occur in normal output.
pub(crate) const PROMPT: &str = "\u{1}CBP> ";
/// [`PROMPT`] as a `mongosh` statement (verified on piped stdin with mongosh 2.14.1: every later
/// prompt, also after `use <db>`, is exactly this token).
const PROMPT_JS: &str = "prompt = () => \"\\u0001CBP> \"; ";
/// On piped stdin `passwordPrompt()` would read the rest of the script as the password (nothing
/// after it runs, and the run looks successful), so it throws instead. Shown as mongosh output.
const PASSWORD_PROMPT_JS: &str = "passwordPrompt = () => { throw new Error(\"passwordPrompt() is not available in the shell tab\") }; ";

/// What to spawn and what to write to its stdin.
///
/// `Debug` is written by hand: the preamble holds the password, so it never prints bytes.
pub struct Invocation {
    /// Always `["--nodb", "--quiet"]`.
    pub args: Vec<OsString>,
    /// One line (ends in a single `\n`) that fixes the prompt ([`PROMPT`]), makes
    /// `passwordPrompt()` throw, connects and prints `__CB_READY__`, or `__CB_FAILED__ <message>`.
    /// Holds the password; written first.
    pub preamble: Vec<u8>,
    /// The user's script as one block ([`script_block`]); written after the ready sentinel, a
    /// line at a time.
    pub script: Vec<u8>,
}

impl std::fmt::Debug for Invocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Invocation")
            .field("args", &self.args)
            .field("preamble", &format_args!("<redacted>"))
            .field("preamble_len", &self.preamble.len())
            .field("script_len", &self.script.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShellError {
    Uri(UriProblem),
    SrvWithTunnel,
    UnverifiableTls,
    EmptyScript,
}

/// Percent-encodes every byte that is not RFC 3986 unreserved (`A-Z a-z 0-9 - . _ ~`).
pub fn percent_encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn build_invocation(
    cfg: &ConnectionConfig,
    password: Option<&str>,
    tunnel: Option<SocketAddr>,
    script: &str,
) -> Result<Invocation, ShellError> {
    if script.trim().is_empty() {
        return Err(ShellError::EmptyScript);
    }
    // Only what a shell needs: name, folder and ssh settings do not matter here.
    cfg.validate_uri()
        .and_then(|()| cfg.validate_x509())
        .map_err(|e| match e {
            CoreError::InvalidUri(problem) => ShellError::Uri(problem),
            _ => ShellError::Uri(UriProblem::Other),
        })?;
    if uri_has_secret_query_param(&cfg.uri) {
        return Err(ShellError::Uri(UriProblem::SecretInUri));
    }
    let uri = connect_uri(cfg, password, tunnel)?;
    let preamble = format!(
        "{PROMPT_JS}{PASSWORD_PROMPT_JS}try {{ db = connect({}); print(\"__CB_READY__\") }} catch (e) {{ print(\"__CB_FAILED__ \" + e.message) }}\n",
        js_string(&uri)
    );
    Ok(Invocation {
        args: vec![OsString::from("--nodb"), OsString::from("--quiet")],
        preamble: preamble.into_bytes(),
        script: script_block(script),
    })
}

/// The script as `mongosh` gets it: one block statement, `{` + the script + a `}` line, with
/// every line break `mongosh`'s line reader ends a line at (`\r\n`, `\r`, U+2028, U+2029) as
/// `\n`, so the lines the runner writes are the lines `mongosh` reads.
///
/// `mongosh` reads piped stdin as REPL input, a line at a time, and evaluates each statement as
/// soon as it is complete: line by line, an `else` on its own line after `}` would run on its
/// own (unconditionally), and a `.map(...)` on the next line would fail. As one block the whole
/// script is one statement, evaluated as one unit. The `{` shares the first line, so the line
/// numbers in `mongosh`'s syntax errors are the editor's.
pub(crate) fn script_block(script: &str) -> Vec<u8> {
    let lines = script
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}'], "\n");
    format!("{{{lines}\n}}\n").into_bytes()
}

/// The stored URI split into the parts we rewrite.
struct UriParts<'a> {
    scheme: &'static str,
    userinfo: Option<&'a str>,
    hosts: &'a str,
    path: &'a str,
    query: Vec<(&'a str, &'a str)>,
}

fn split_uri(uri: &str) -> UriParts<'_> {
    let (scheme, rest) = match uri.strip_prefix(SRV_SCHEME) {
        Some(rest) => (SRV_SCHEME, rest),
        None => (PLAIN_SCHEME, uri.strip_prefix(PLAIN_SCHEME).unwrap_or(uri)),
    };
    let (head, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = head.split_once('/').unwrap_or((head, ""));
    let (userinfo, hosts) = match authority.rsplit_once('@') {
        Some((user, hosts)) => (Some(user), hosts),
        None => (None, authority),
    };
    let query = query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| kv.split_once('=').unwrap_or((kv, "")))
        .collect();
    UriParts {
        scheme,
        userinfo,
        hosts,
        path,
        query,
    }
}

fn query_is_true(query: &[(&str, &str)], keys: &[&str]) -> bool {
    query
        .iter()
        .any(|(k, v)| keys.iter().any(|key| k.eq_ignore_ascii_case(key)) && *v == "true")
}

fn connect_uri(
    cfg: &ConnectionConfig,
    password: Option<&str>,
    tunnel: Option<SocketAddr>,
) -> Result<String, ShellError> {
    let parts = split_uri(&cfg.uri);
    let mut options = OptionList::from(&parts.query);
    let hosts = match tunnel {
        Some(addr) => {
            if parts.scheme == SRV_SCHEME {
                return Err(ShellError::SrvWithTunnel);
            }
            let tls_on = cfg.tls.enabled || query_is_true(&parts.query, &["tls", "ssl"]);
            let relaxed = cfg.tls.allow_invalid_certs
                || query_is_true(
                    &parts.query,
                    &["tlsAllowInvalidCertificates", "tlsInsecure"],
                );
            if tls_on && tls_blocks_tunnel(relaxed.then_some(true)) {
                return Err(ShellError::UnverifiableTls);
            }
            options.set("directConnection", "true");
            addr.to_string()
        }
        None => parts.hosts.to_owned(),
    };
    apply_auth_options(&mut options, cfg);
    apply_tls_options(&mut options, cfg);
    let userinfo = userinfo(cfg, password, parts.userinfo);
    let query = options.render();
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    Ok(format!(
        "{}{userinfo}{hosts}/{}{query}",
        parts.scheme, parts.path
    ))
}

/// `user[:password]@`, encoded. With no auth mechanism the stored URI's own user (it never has a
/// password) is kept as is, like the driver path does.
fn userinfo(cfg: &ConnectionConfig, password: Option<&str>, stored: Option<&str>) -> String {
    if cfg.auth == AuthMechanism::None {
        return stored.map_or_else(String::new, |u| format!("{u}@"));
    }
    let user = match (&cfg.username, stored) {
        (Some(user), _) => percent_encode_component(user),
        (None, Some(stored)) => stored.to_owned(),
        (None, None) => return String::new(),
    };
    match password {
        Some(pw) => format!("{user}:{}@", percent_encode_component(pw)),
        None => format!("{user}@"),
    }
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

fn apply_auth_options(options: &mut OptionList, cfg: &ConnectionConfig) {
    let Some(mechanism) = mechanism_name(cfg.auth) else {
        return;
    };
    options.set("authMechanism", mechanism);
    if let Some(source) = effective_auth_source(cfg) {
        options.set("authSource", &percent_encode_component(&source));
    }
}

fn apply_tls_options(options: &mut OptionList, cfg: &ConnectionConfig) {
    if !cfg.tls.enabled {
        return;
    }
    options.set("tls", "true");
    if let Some(ca) = &cfg.tls.ca_file {
        options.set("tlsCAFile", &encode_path(ca));
    }
    if let Some(cert) = &cfg.tls.client_cert_key_file {
        options.set("tlsCertificateKeyFile", &encode_path(cert));
    }
    if cfg.tls.allow_invalid_certs {
        options.set("tlsAllowInvalidCertificates", "true");
    }
}

fn encode_path(path: &Path) -> String {
    percent_encode_component(&path.to_string_lossy())
}

/// Query options in order; `set` replaces any existing key (case-insensitive), so the config wins
/// over what the stored URI said.
struct OptionList(Vec<(String, String)>);

impl OptionList {
    fn from(query: &[(&str, &str)]) -> Self {
        Self(
            query
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }

    fn set(&mut self, key: &str, value: &str) {
        self.0.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        self.0.push((key.to_owned(), value.to_owned()));
    }

    fn render(&self) -> String {
        self.0
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&")
    }
}

#[cfg(test)]
#[path = "invocation_tests.rs"]
mod tests;
