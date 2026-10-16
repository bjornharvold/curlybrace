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
    // Guards against a stored host that differs from what was typed. The conversion silently
    // DROPS characters it maps to nothing (variation selectors, soft hyphen, the zero-width
    // space, Mongolian free variation selectors, ...), so every non-ASCII character is converted
    // alone and refused when that yields an empty string: the answer comes from the converter's
    // own table, so it cannot lag behind Unicode. The shared forbidden-label set additionally
    // refuses control, bidi and zero-width characters, which the converter rejects or keeps.
    // The two joiners are exempt from both: they are legitimate inside Persian and Indic names,
    // the converter's own joiner rules judge their context, and `keeps_joiners` proves the
    // result still holds every one that was typed.
    let refused =
        |c: char| !is_joiner(c) && (is_forbidden_label_char(c) || (!c.is_ascii() && is_dropped(c)));
    if host.len() > MAX_UNICODE_HOST_BYTES || host.chars().any(refused) {
        return Err(UriProblem::BadHost);
    }
    let ascii = Uts46::new()
        .to_ascii(
            host.as_bytes(),
            AsciiDenyList::EMPTY,
            Hyphens::Allow,
            DnsLength::VerifyAllowRootDot,
        )
        .map(|ascii| ascii.into_owned())
        .map_err(|_| UriProblem::BadHost)?;
    if keeps_joiners(host, &ascii) {
        Ok(ascii)
    } else {
        Err(UriProblem::BadHost)
    }
}

/// U+200C ZERO WIDTH NON-JOINER and U+200D ZERO WIDTH JOINER.
fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// Whether the converted host, read back as Unicode, holds as many joiners as were typed, so a
/// joiner the conversion dropped can never be stored without the person's knowledge.
fn keeps_joiners(typed: &str, ascii: &str) -> bool {
    let wanted = typed.chars().filter(|c| is_joiner(*c)).count();
    if wanted == 0 {
        return true;
    }
    let (back, result) =
        Uts46::new().to_unicode(ascii.as_bytes(), AsciiDenyList::EMPTY, Hyphens::Allow);
    result.is_ok() && back.chars().filter(|c| is_joiner(*c)).count() == wanted
}

/// Whether the conversion maps this character to nothing when it stands alone. A character that
/// fails on its own (a lone combining mark) is not dropped: the full conversion judges it in
/// context.
fn is_dropped(c: char) -> bool {
    let mut buf = [0u8; 4];
    let one = c.encode_utf8(&mut buf);
    Uts46::new()
        .to_ascii(
            one.as_bytes(),
            AsciiDenyList::EMPTY,
            Hyphens::Allow,
            DnsLength::Ignore,
        )
        .is_ok_and(|ascii| ascii.is_empty())
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
