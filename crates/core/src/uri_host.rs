//! Host-list checks for connection strings. Every failure is a `UriProblem` (fixed text, never
//! any part of the input).
use crate::connection::is_forbidden_label_char;
use crate::uri::percent_decode;
use crate::uri_problem::UriProblem;
use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

type Check<T> = Result<T, UriProblem>;

/// How hosts are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostMode {
    /// Save and import: a host is accepted only in the form the driver accepts (ASCII letters,
    /// digits, `.`, `_`, `-`). Nothing is decoded or rewritten.
    Strict,
    /// From URI: a percent-encoded host is decoded and a Unicode host converted to punycode, so
    /// the stored string is the ASCII form the driver needs.
    Normalize,
}

/// The longest Unicode host handed to the converter. A DNS name is at most 253 bytes; this leaves
/// room for multi-byte characters while keeping hostile input cheap.
const MAX_UNICODE_HOST_BYTES: usize = 1024;

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(is_name_byte)
}

/// Checks the comma-separated host list of a connection string and returns it in the form to
/// store: unchanged in `Strict` mode, with every entry normalised in `Normalize` mode.
pub(crate) fn validate_hosts(hosts: &str, srv: bool, mode: HostMode) -> Check<String> {
    let entries: Vec<&str> = hosts.split(',').collect();
    if srv && entries.len() != 1 {
        return Err(UriProblem::SrvRules);
    }
    let checked = entries
        .iter()
        .map(|e| validate_host(e, srv, mode))
        .collect::<Check<Vec<String>>>()?;
    Ok(checked.join(","))
}

fn validate_host(entry: &str, srv: bool, mode: HostMode) -> Check<String> {
    if entry.starts_with("%2F") || entry.starts_with("%2f") {
        let ok = !srv && entry.ends_with(".sock") && is_encoded_path(entry);
        return if ok {
            Ok(entry.to_owned())
        } else {
            Err(UriProblem::BadHost)
        };
    }
    if let Some(inner) = entry.strip_prefix('[') {
        let (addr, after) = inner.split_once(']').ok_or(UriProblem::BadHost)?;
        let addr_ok = !addr.is_empty()
            && addr
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or(UriProblem::BadHost)?),
        };
        if !addr_ok {
            return Err(UriProblem::BadHost);
        }
        check_port_of(port, srv)?;
        return Ok(entry.to_owned());
    }
    let (host, port) = match entry.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (entry, None),
    };
    let host = match mode {
        HostMode::Strict => strict_host(host)?,
        HostMode::Normalize => normalized_host(host)?,
    };
    check_port_of(port, srv)?;
    Ok(match port {
        Some(p) => format!("{host}:{p}"),
        None => host,
    })
}

fn check_port_of(port: Option<&str>, srv: bool) -> Check<()> {
    match port {
        None => Ok(()),
        Some(_) if srv => Err(UriProblem::SrvRules),
        Some(p) => check_port(p),
    }
}

/// A host exactly as the driver takes it. Unicode is told apart so the hint can point to From URI.
fn strict_host(host: &str) -> Check<String> {
    if !host.is_ascii() {
        Err(UriProblem::NonAsciiHost)
    } else if is_name(host) {
        Ok(host.to_owned())
    } else {
        Err(UriProblem::BadHost)
    }
}

/// Percent-decodes a host, converts Unicode to punycode, then holds the result to the strict
/// rules, so a decoded `/`, `@`, `:`, space or control character (or one the conversion maps to
/// such a character) is a bad host.
fn normalized_host(host: &str) -> Check<String> {
    if host.is_empty() {
        return Err(UriProblem::BadHost);
    }
    let decoded = if host.contains('%') {
        percent_decode(host)?
    } else {
        host.to_owned()
    };
    let ascii = if decoded.is_ascii() {
        decoded
    } else {
        to_punycode(&decoded)?
    };
    if is_name(&ascii) {
        Ok(ascii)
    } else {
        Err(UriProblem::BadHost)
    }
}

fn to_punycode(host: &str) -> Check<String> {
    // The conversion silently drops invisible characters (zero-width marks, soft hyphens): refuse
    // them instead of storing a host that differs from what was typed.
    if host.len() > MAX_UNICODE_HOST_BYTES || host.chars().any(is_forbidden_label_char) {
        return Err(UriProblem::BadHost);
    }
    Uts46::new()
        .to_ascii(
            host.as_bytes(),
            AsciiDenyList::EMPTY,
            Hyphens::Allow,
            DnsLength::Verify,
        )
        .map(|ascii| ascii.into_owned())
        .map_err(|_| UriProblem::BadHost)
}

fn check_port(p: &str) -> Check<()> {
    let digits = (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit());
    match p.parse::<u16>() {
        Ok(n) if digits && n > 0 => Ok(()),
        _ => Err(UriProblem::BadPort),
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
