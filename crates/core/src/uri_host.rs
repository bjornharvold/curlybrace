//! Host-list checks for connection strings. Every failure is a `UriProblem` (fixed text, never
//! any part of the input).
use crate::uri_problem::UriProblem;

type Check<T> = Result<T, UriProblem>;

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

/// Checks the comma-separated host list of a connection string.
pub(crate) fn validate_hosts(hosts: &str, srv: bool) -> Check<()> {
    let entries: Vec<&str> = hosts.split(',').collect();
    if srv && entries.len() != 1 {
        return Err(UriProblem::SrvRules);
    }
    entries.iter().try_for_each(|e| validate_host(e, srv))
}

fn validate_host(entry: &str, srv: bool) -> Check<()> {
    if entry.starts_with("%2F") || entry.starts_with("%2f") {
        let ok = !srv && entry.ends_with(".sock") && is_encoded_path(entry);
        return if ok { Ok(()) } else { Err(UriProblem::BadHost) };
    }
    let (host_ok, port) = if let Some(inner) = entry.strip_prefix('[') {
        let (addr, after) = inner.split_once(']').ok_or(UriProblem::BadHost)?;
        let addr_ok = !addr.is_empty()
            && addr
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or(UriProblem::BadHost)?),
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
        return Err(UriProblem::BadHost);
    }
    match port {
        None => Ok(()),
        Some(_) if srv => Err(UriProblem::SrvRules),
        Some(p) => check_port(p),
    }
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
