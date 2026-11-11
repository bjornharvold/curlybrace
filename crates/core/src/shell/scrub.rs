//! Masks the connection password in `mongosh` output before it is shown. Best-effort courtesy,
//! not a security boundary: output arrives in chunks, so a secret can be split across two pushes.

/// What a secret is replaced with.
pub const MASK: &str = "••••";

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
