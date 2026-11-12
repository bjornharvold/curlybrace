//! Which bytes of a mongosh script are code. One pass that mirrors the lexical rules of
//! [`format_script`](super::shell_format::format_script) (and the highlighter's lexer): strings
//! end at the line end unless it is escaped, templates and block comments span lines, a `/` is a
//! regex literal where [`starts_regex`] says so. The three are separate implementations that
//! mirror one another, kept in step by a parity test, not one shared piece of code.
//! Pure and egui-free; no user-facing strings.
use super::shell_format::{Lex, regex_end, starts_regex};

/// A literal that is not code: what an unterminated one is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Literal {
    String,
    Template,
    Comment,
}

/// Which bytes of a script are code, and which literals never end.
pub(crate) struct CodeScan {
    /// Per byte: code, as opposed to a string, template text, comment or regex literal
    /// (delimiters included). A template's `${` is not code but the `{`, its contents and its
    /// closing `}` are, so brackets balance there like anywhere else.
    pub in_code: Vec<bool>,
    /// Literals left open, as (kind, byte offset of the opening delimiter), in no set order.
    pub unterminated: Vec<(Literal, usize)>,
}

/// Where a `\` at `i` leaves the scan: past the escaped byte (`\r\n` counts as one), and the
/// start of the line the scan is then on.
fn skip_escape(bytes: &[u8], i: usize, line_start: usize) -> (usize, usize) {
    match (bytes.get(i + 1), bytes.get(i + 2)) {
        (Some(b'\r'), Some(b'\n')) => (i + 3, i + 3),
        (Some(b'\n'), _) => (i + 2, i + 2),
        _ => (i + 2, line_start),
    }
}

/// The scan's running state; each `step_*` handles the byte at `i` and returns the next index.
struct Walk<'a> {
    bytes: &'a [u8],
    in_code: Vec<bool>,
    unterminated: Vec<(Literal, usize)>,
    state: Lex,
    line_start: usize,
    /// The end of the line starting at the first byte, once a slash asked for it.
    line_end: Option<(usize, usize)>,
    /// Where the literal being scanned opened.
    lit_start: usize,
    depth: usize,
    /// For each unfinished `${`: the bracket depth it was entered at and where its template began.
    subst: Vec<(usize, usize)>,
}

impl Walk<'_> {
    /// A newline ends a line comment and an unterminated string. The newline that ends a line is
    /// code, unlike one inside a template or block comment.
    fn step_newline(&mut self, i: usize) -> usize {
        match self.state {
            Lex::Single | Lex::Double => {
                self.unterminated.push((Literal::String, self.lit_start));
                self.state = Lex::Code;
            }
            Lex::LineComment => self.state = Lex::Code,
            _ => {}
        }
        self.in_code[i] = self.state == Lex::Code;
        self.line_start = i + 1;
        i + 1
    }

    fn step_code(&mut self, i: usize) -> usize {
        let b = self.bytes[i];
        let next = self.bytes.get(i + 1).copied();
        self.in_code[i] = true;
        match b {
            b'\'' | b'"' | b'`' => {
                self.open_literal(i, Self::quote_state(b));
                i + 1
            }
            b'/' if next == Some(b'/') => {
                self.open_literal(i, Lex::LineComment);
                i + 2
            }
            b'/' if next == Some(b'*') => {
                self.open_literal(i, Lex::BlockComment);
                i + 2
            }
            b'/' => self.step_slash(i),
            b'{' | b'[' | b'(' => {
                self.depth += 1;
                i + 1
            }
            b'}' | b']' | b')' => {
                self.close_bracket(b);
                i + 1
            }
            _ => i + 1,
        }
    }

    fn quote_state(quote: u8) -> Lex {
        match quote {
            b'\'' => Lex::Single,
            b'"' => Lex::Double,
            _ => Lex::Template,
        }
    }

    fn open_literal(&mut self, i: usize, state: Lex) {
        self.in_code[i] = false;
        self.lit_start = i;
        self.state = state;
    }

    /// A `/` that is not a comment: a regex literal ends at its closing `/` on this line; a
    /// division is code.
    fn step_slash(&mut self, i: usize) -> usize {
        let from = i - self.line_start;
        if !starts_regex(&self.bytes[self.line_start..], from) {
            return i + 1;
        }
        let line_end = self.line_end();
        match regex_end(&self.bytes[self.line_start..line_end], from) {
            Some(end) => {
                self.in_code[i] = false;
                self.line_start + end + 1
            }
            None => i + 1,
        }
    }

    /// The end of the line that starts at `line_start` (its newline, or the text's end), found
    /// once per line: searching from every slash would be quadratic on one long line.
    fn line_end(&mut self) -> usize {
        match self.line_end {
            Some((start, end)) if start == self.line_start => end,
            _ => {
                let end = self.bytes[self.line_start..]
                    .iter()
                    .position(|&c| c == b'\n')
                    .map_or(self.bytes.len(), |n| self.line_start + n);
                self.line_end = Some((self.line_start, end));
                end
            }
        }
    }

    /// A closing `}` at the depth a `${` was entered at resumes the template.
    fn close_bracket(&mut self, b: u8) {
        self.depth = self.depth.saturating_sub(1);
        if b == b'}' && self.subst.last().is_some_and(|&(d, _)| d == self.depth) {
            self.lit_start = self.subst.pop().map_or(0, |(_, start)| start);
            self.state = Lex::Template;
        }
    }

    fn step_text(&mut self, i: usize) -> usize {
        let b = self.bytes[i];
        let quote = match self.state {
            Lex::Single => b'\'',
            Lex::Double => b'"',
            _ => b'`',
        };
        if b == b'\\' {
            let (next, line_start) = skip_escape(self.bytes, i, self.line_start);
            self.line_start = line_start;
            return next;
        }
        if b == quote {
            self.state = Lex::Code;
        } else if self.state == Lex::Template && b == b'$' && self.bytes.get(i + 1) == Some(&b'{') {
            self.subst.push((self.depth, self.lit_start));
            self.depth += 1;
            self.in_code[i + 1] = true;
            self.state = Lex::Code;
            return i + 2;
        }
        i + 1
    }

    fn step_block_comment(&mut self, i: usize) -> usize {
        if self.bytes[i] == b'*' && self.bytes.get(i + 1) == Some(&b'/') {
            self.state = Lex::Code;
            return i + 2;
        }
        i + 1
    }

    fn finish(mut self) -> CodeScan {
        match self.state {
            Lex::Single | Lex::Double => self.unterminated.push((Literal::String, self.lit_start)),
            Lex::Template => self.unterminated.push((Literal::Template, self.lit_start)),
            Lex::BlockComment => self.unterminated.push((Literal::Comment, self.lit_start)),
            Lex::Code | Lex::LineComment => {}
        }
        CodeScan {
            in_code: self.in_code,
            unterminated: self.unterminated,
        }
    }
}

/// One pass over `text`; see the module docs for the rules it follows.
pub(crate) fn scan_code(text: &str) -> CodeScan {
    let bytes = text.as_bytes();
    let mut walk = Walk {
        bytes,
        in_code: vec![false; bytes.len()],
        unterminated: Vec::new(),
        state: Lex::Code,
        line_start: 0,
        line_end: None,
        lit_start: 0,
        depth: 0,
        subst: Vec::new(),
    };
    let mut i = 0;
    while i < bytes.len() {
        i = if bytes[i] == b'\n' {
            walk.step_newline(i)
        } else {
            match walk.state {
                Lex::Code => walk.step_code(i),
                Lex::Single | Lex::Double | Lex::Template => walk.step_text(i),
                Lex::LineComment => i + 1,
                Lex::BlockComment => walk.step_block_comment(i),
            }
        };
    }
    walk.finish()
}

#[cfg(test)]
#[path = "shell_scan_tests.rs"]
mod tests;
