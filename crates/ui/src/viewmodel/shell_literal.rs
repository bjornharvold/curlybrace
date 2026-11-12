//! A mongosh object literal read as a BSON document, for the Shell tab's index-miss guard. Only
//! plain data is accepted: bare or quoted keys, single- or double-quoted strings, numbers,
//! booleans, `null`, nested objects and arrays, `ObjectId("<24 hex>")` and
//! `ISODate("<ISO 8601>")`, with whitespace and comments between them and trailing commas.
//! Anything else (an identifier as a value, a call, a spread, a computed key, a template or regex
//! literal, `new`, trailing text) makes the whole text `None`: the guard never guesses.
//!
//! Numbers: an integer literal (no `.` or exponent) is an `Int32` when it fits, else an `Int64`
//! when it fits, else a `Double`; every other number is a `Double`. This does not mirror mongosh,
//! which sends a large integer as a `Double` and loses precision above 2^53; the guard only
//! asks which index a query would use, and that does not depend on the numeric type. A legacy
//! octal (`010`, which mongosh reads as 8) is rejected, in values and in bare numeric keys.
//! Bare keys are ASCII only (quoted keys may hold anything).
//!
//! Total on any input: every step advances or fails, nesting is cut off at [`MAX_DEPTH`], and
//! text is read by char, never sliced inside one. Pure and egui-free; no user-facing strings.
use bson::{Bson, DateTime, Document, oid::ObjectId};

/// The deepest nesting of objects and arrays read; the outer object is depth 1.
pub const MAX_DEPTH: usize = 64;

/// `text` as one object literal (surrounding whitespace and comments allowed), or `None`.
pub fn parse_literal(text: &str) -> Option<Document> {
    whole(text, |p| p.object(1))
}

/// `text` as one single- or double-quoted string literal, or `None`.
pub(crate) fn parse_string(text: &str) -> Option<String> {
    whole(text, |p| p.string())
}

/// Whether `text` holds only whitespace and complete comments.
pub(crate) fn is_blank(text: &str) -> bool {
    whole(text, |_| Some(())).is_some()
}

/// `read` over all of `text`, with only whitespace and comments around it.
fn whole<T>(text: &str, read: impl FnOnce(&mut Parser<'_>) -> Option<T>) -> Option<T> {
    let mut parser = Parser { text, pos: 0 };
    parser.skip_trivia()?;
    let value = read(&mut parser)?;
    parser.skip_trivia()?;
    (parser.pos == text.len()).then_some(value)
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_ident_byte(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

struct Parser<'a> {
    text: &'a str,
    /// Always on a char boundary: it only moves past ASCII bytes or whole chars.
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.text.as_bytes().get(self.pos + ahead).copied()
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        (self.peek() == Some(b)).then(|| self.pos += 1)
    }

    /// Skips whitespace and comments; `None` for a block comment that never ends.
    fn skip_trivia(&mut self) -> Option<()> {
        loop {
            match (self.peek(), self.peek_at(1)) {
                (Some(b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c), _) => self.pos += 1,
                (Some(b'/'), Some(b'/')) => {
                    let rest = self.text.get(self.pos..)?;
                    self.pos += rest.find('\n').unwrap_or(rest.len());
                }
                (Some(b'/'), Some(b'*')) => {
                    let rest = self.text.get(self.pos + 2..)?;
                    self.pos += 2 + rest.find("*/")? + 2;
                }
                _ => return Some(()),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Option<Document> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.eat(b'{')?;
        let mut doc = Document::new();
        loop {
            self.skip_trivia()?;
            if self.eat(b'}').is_some() {
                return Some(doc);
            }
            let key = self.key()?;
            self.skip_trivia()?;
            self.eat(b':')?;
            self.skip_trivia()?;
            let value = self.value(depth)?;
            doc.insert(key, value);
            if !self.separator(b'}')? {
                return Some(doc);
            }
        }
    }

    fn array(&mut self, depth: usize) -> Option<Vec<Bson>> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.eat(b'[')?;
        let mut items = Vec::new();
        loop {
            self.skip_trivia()?;
            if self.eat(b']').is_some() {
                return Some(items);
            }
            items.push(self.value(depth)?);
            if !self.separator(b']')? {
                return Some(items);
            }
        }
    }

    /// After an entry: `Some(true)` past a `,` (more may follow), `Some(false)` past `close`.
    fn separator(&mut self, close: u8) -> Option<bool> {
        self.skip_trivia()?;
        if self.eat(b',').is_some() {
            return Some(true);
        }
        self.eat(close).map(|()| false)
    }

    fn key(&mut self) -> Option<String> {
        match self.peek()? {
            b'\'' | b'"' => self.string(),
            // A bare key is ASCII only: JavaScript reads some non-ASCII chars (a no-break
            // space) as whitespace, so the field name would be wrong.
            b if is_ident_start(b) => self.ident().filter(|k| k.is_ascii()).map(str::to_string),
            b if b.is_ascii_digit() => {
                self.reject_legacy_octal()?;
                let start = self.pos;
                self.digits();
                self.text.get(start..self.pos).map(str::to_string)
            }
            _ => None,
        }
    }

    /// A value inside an object or array at `depth`.
    fn value(&mut self, depth: usize) -> Option<Bson> {
        match self.peek()? {
            b'{' => self.object(depth + 1).map(Bson::Document),
            b'[' => self.array(depth + 1).map(Bson::Array),
            b'\'' | b'"' => self.string().map(Bson::String),
            b'-' | b'+' | b'.' => self.number(),
            b if b.is_ascii_digit() => self.number(),
            b if is_ident_start(b) => self.word(),
            _ => None,
        }
    }

    fn ident(&mut self) -> Option<&str> {
        let start = self.pos;
        while self.peek().is_some_and(is_ident_byte) {
            self.pos += 1;
        }
        // Only ASCII bytes end an identifier, so `pos` is a char boundary; `get` checks anyway.
        self.text.get(start..self.pos).filter(|s| !s.is_empty())
    }

    /// `true`, `false`, `null`, `ObjectId(...)` or `ISODate(...)`.
    fn word(&mut self) -> Option<Bson> {
        match self.ident()? {
            "true" => Some(Bson::Boolean(true)),
            "false" => Some(Bson::Boolean(false)),
            "null" => Some(Bson::Null),
            "ObjectId" => {
                let hex = self.string_argument()?;
                ObjectId::parse_str(hex).ok().map(Bson::ObjectId)
            }
            "ISODate" => {
                let text = self.string_argument()?;
                iso_date(&text).map(Bson::DateTime)
            }
            _ => None,
        }
    }

    /// `( "<string>" )` right after a function name.
    fn string_argument(&mut self) -> Option<String> {
        self.skip_trivia()?;
        self.eat(b'(')?;
        self.skip_trivia()?;
        let text = self.string()?;
        self.skip_trivia()?;
        self.eat(b')')?;
        Some(text)
    }

    /// `None` at a `0` followed by a digit: sloppy-mode mongosh reads `010` as octal 8.
    fn reject_legacy_octal(&self) -> Option<()> {
        let octal =
            self.peek() == Some(b'0') && self.peek_at(1).is_some_and(|b| b.is_ascii_digit());
        (!octal).then_some(())
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        self.pos - start
    }

    /// A decimal number with an optional sign, fraction and exponent; see the module docs.
    fn number(&mut self) -> Option<Bson> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.pos += 1;
        }
        self.reject_legacy_octal()?;
        let mut mantissa = self.digits();
        let mut integer = true;
        if self.eat(b'.').is_some() {
            integer = false;
            mantissa += self.digits();
        }
        if mantissa == 0 {
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            integer = false;
            self.pos += 1;
            if matches!(self.peek(), Some(b'-' | b'+')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return None;
            }
        }
        let text = self.text.get(start..self.pos)?;
        if integer {
            if let Ok(n) = text.parse::<i32>() {
                return Some(Bson::Int32(n));
            }
            if let Ok(n) = text.parse::<i64>() {
                return Some(Bson::Int64(n));
            }
        }
        text.parse::<f64>().ok().map(Bson::Double)
    }

    /// A single- or double-quoted string with JavaScript escapes; a raw line break ends it
    /// unterminated, which is `None`.
    fn string(&mut self) -> Option<String> {
        let quote = self.peek().filter(|&b| b == b'\'' || b == b'"')?;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let c = self.next_char()?;
            match c {
                '\n' | '\r' => return None,
                '\\' => {
                    if let Some(escaped) = self.escape()? {
                        out.push(escaped);
                    }
                }
                c if c as u32 == u32::from(quote) => return Some(out),
                c => out.push(c),
            }
        }
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.text.get(self.pos..)?.chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    /// The char after a `\`: `Some(None)` for a line continuation, `None` when malformed.
    fn escape(&mut self) -> Option<Option<char>> {
        let c = self.next_char()?;
        let escaped = match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'v' => '\u{b}',
            '0' if !self.peek().is_some_and(|b| b.is_ascii_digit()) => '\0',
            'x' => char::from_u32(self.hex(2)?)?,
            'u' => self.unicode_escape()?,
            '\n' => return Some(None),
            '\r' => {
                self.eat(b'\n');
                return Some(None);
            }
            c if c.is_ascii_digit() => return None,
            c => c,
        };
        Some(Some(escaped))
    }

    /// After `\u`: `{hex}`, or four hex digits, joined with a following low surrogate escape.
    fn unicode_escape(&mut self) -> Option<char> {
        if self.eat(b'{').is_some() {
            let start = self.pos;
            while self.peek().is_some_and(|b| b.is_ascii_hexdigit()) && self.pos - start < 6 {
                self.pos += 1;
            }
            let code = u32::from_str_radix(self.text.get(start..self.pos)?, 16).ok()?;
            self.eat(b'}')?;
            return char::from_u32(code);
        }
        let high = self.hex(4)?;
        if !(0xD800..0xDC00).contains(&high) {
            return char::from_u32(high);
        }
        if self.peek() != Some(b'\\') || self.peek_at(1) != Some(b'u') {
            return None;
        }
        self.pos += 2;
        let low = self.hex(4)?;
        if !(0xDC00..0xE000).contains(&low) {
            return None;
        }
        char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00))
    }

    fn hex(&mut self, count: usize) -> Option<u32> {
        let digits = self.text.get(self.pos..self.pos + count)?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.pos += count;
        u32::from_str_radix(digits, 16).ok()
    }
}

/// An `ISODate` argument: RFC 3339, or the same without its zone (read as UTC), or a date alone
/// (midnight UTC).
fn iso_date(text: &str) -> Option<DateTime> {
    [
        text.to_string(),
        format!("{text}Z"),
        format!("{text}T00:00:00Z"),
    ]
    .iter()
    .find_map(|candidate| DateTime::parse_rfc3339_str(candidate).ok())
}

#[cfg(test)]
#[path = "shell_literal_tests.rs"]
mod tests;
