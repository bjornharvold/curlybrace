//! Masks the connection password in `mongosh` output before it is shown. Best-effort courtesy,
//! not a security boundary: output arrives in chunks, so a secret can be split across two pushes.

use super::invocation::percent_encode_component;

/// What a secret is replaced with.
pub const MASK: &str = "••••";

/// Every text the password can show up as, deduplicated, in a stable order: raw, percent-encoded
/// (as in a connection string), JSON-escaped (what `EJSON.stringify` writes between the quotes)
/// and as `mongosh` echoes a string (Node's `util.inspect`, with and without the escaped quote).
/// Empty for an empty password.
pub fn secret_forms(password: &str) -> Vec<String> {
    if password.is_empty() {
        return Vec::new();
    }
    let candidates = [
        password.to_owned(),
        percent_encode_component(password),
        json_escaped(password),
        inspect_escaped(password, false),
        inspect_escaped(password, true),
    ];
    candidates.into_iter().fold(Vec::new(), |forms, candidate| {
        if candidate.is_empty() || forms.contains(&candidate) {
            forms
        } else {
            forms.into_iter().chain([candidate]).collect()
        }
    })
}

/// The contents of the JSON string for `text`, without the surrounding quotes.
fn json_escaped(text: &str) -> String {
    // Serializing a `&str` cannot fail; the fallback keeps the raw text (still masked as the raw form).
    let quoted = serde_json::to_string(text).unwrap_or_default();
    quoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or_default()
        .to_owned()
}

/// The contents of a string as Node's `util.inspect` prints it: backslash doubled, control
/// characters as `\n`, `\t`... or `\xNN`, and, in its default single-quote style, `'` as `\'`
/// (`escape_single_quote`). Inspect picks double quotes or backticks when the text contains `'`;
/// those styles leave the quote alone, hence both variants.
fn inspect_escaped(text: &str, escape_single_quote: bool) -> String {
    text.chars()
        .fold(String::with_capacity(text.len()), |mut out, ch| {
            match ch {
                '\\' => out.push_str("\\\\"),
                '\'' if escape_single_quote => out.push_str("\\'"),
                '\u{8}' => out.push_str("\\b"),
                '\t' => out.push_str("\\t"),
                '\n' => out.push_str("\\n"),
                '\u{c}' => out.push_str("\\f"),
                '\r' => out.push_str("\\r"),
                '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => {
                    out.push_str(&format!("\\x{:02X}", u32::from(ch)));
                }
                _ => out.push(ch),
            }
            out
        })
}

/// Streaming secret masker for one output stream.
#[derive(Debug)]
pub struct Scrubber {
    /// Non-empty, unique secrets, longest first so the longer one wins when one contains another.
    secrets: Vec<String>,
    /// Unshown tail of the text so far: a proper prefix of at least one secret.
    carry: String,
}

impl Scrubber {
    pub fn new(secrets: &[&str]) -> Self {
        let mut secrets: Vec<String> = secrets
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| (*s).to_owned())
            .collect();
        secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        secrets.dedup();
        Self {
            secrets,
            carry: String::new(),
        }
    }

    /// Text that is safe to show now. Holds back a tail only while it could still become a secret.
    pub fn push(&mut self, chunk: &str) -> String {
        self.scan(chunk, false)
    }

    /// Releases the held tail, masking a complete secret in it; call when the stream ends (the
    /// scrubber can then go on with a new stream).
    pub fn finish(&mut self) -> String {
        self.scan("", true)
    }

    /// `text` with every secret in it masked, as a whole: nothing is held back, and the tail this
    /// scrubber holds is neither used nor changed.
    pub fn mask(&self, text: &str) -> String {
        let mut whole = Self {
            secrets: self.secrets.clone(),
            carry: String::new(),
        };
        whole.scan(text, true)
    }

    /// Scans carry + chunk. Unless `at_end`, holds back a tail that could still become a secret.
    fn scan(&mut self, chunk: &str, at_end: bool) -> String {
        let text = std::mem::take(&mut self.carry) + chunk;
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < text.len() {
            let rest = &text[i..];
            if !at_end && self.could_become_secret(rest) {
                self.carry = rest.to_owned();
                return out;
            }
            match self.secrets.iter().find(|s| rest.starts_with(s.as_str())) {
                Some(secret) => {
                    out.push_str(MASK);
                    i += secret.len();
                }
                None => {
                    let ch = rest.chars().next().expect("rest is non-empty");
                    out.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        out
    }

    /// True when `rest` is a strict prefix of some secret, so more input may complete it.
    fn could_become_secret(&self, rest: &str) -> bool {
        self.secrets
            .iter()
            .any(|s| s.len() > rest.len() && s.starts_with(rest))
    }
}

#[cfg(test)]
#[path = "scrub_tests.rs"]
mod tests;
