//! Streaming UTF-8 decoding of child output. A character split across two reads is kept whole,
//! so a non-ASCII secret split that way still reaches the scrubber intact.

const REPLACEMENT: char = '\u{FFFD}';

/// Decodes one byte stream; invalid bytes become U+FFFD.
#[derive(Debug, Default)]
pub(crate) struct Utf8Decoder {
    /// An incomplete sequence at the end of the last push (at most three bytes).
    pending: Vec<u8>,
}

impl Utf8Decoder {
    /// The text decodable so far; holds back an unfinished trailing character.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> String {
        let joined;
        let data: &[u8] = if self.pending.is_empty() {
            bytes
        } else {
            joined = [std::mem::take(&mut self.pending).as_slice(), bytes].concat();
            &joined
        };
        let mut out = String::with_capacity(data.len());
        let mut rest = data;
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    out.push_str(text);
                    return out;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    out.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match error.error_len() {
                        Some(len) => {
                            out.push(REPLACEMENT);
                            rest = &after[len..];
                        }
                        None => {
                            self.pending = after.to_vec();
                            return out;
                        }
                    }
                }
            }
        }
    }

    /// Ends the stream: an unfinished character becomes one U+FFFD.
    pub(crate) fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        self.pending.clear();
        REPLACEMENT.to_string()
    }
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
