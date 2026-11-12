//! The tokens [`read_calls`](super::shell_queries::read_calls) walks: the code of a script cut
//! into identifiers and punctuation, with comments dropped and every other non-code run (a
//! string, template text, a regex literal) kept as one opaque token. Brackets are paired in the
//! same pass, so finding a closing bracket never searches. Built on
//! [`scan_code`](super::shell_scan::scan_code), never re-deciding what is code.
use super::shell_scan::scan_code;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A run of identifier bytes (letters, digits, `_`, `$`, any non-ASCII).
    Ident,
    /// One byte of other code.
    Punct(u8),
    /// A string, template text or regex literal.
    Opaque,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Token {
    pub kind: Kind,
    /// Byte offsets in the original text; always on char boundaries.
    pub start: usize,
    pub end: usize,
    /// First token on its line (or of the text).
    pub line_start: bool,
    /// For a paired bracket, the index of its partner.
    pub partner: Option<usize>,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Builds the token list; see the module docs.
struct Lexer<'a> {
    bytes: &'a [u8],
    tokens: Vec<Token>,
    new_line: bool,
}

impl Lexer<'_> {
    fn push(&mut self, kind: Kind, start: usize, end: usize) {
        self.tokens.push(Token {
            kind,
            start,
            end,
            line_start: self.new_line,
            partner: None,
        });
        self.new_line = false;
    }

    /// A run of code bytes: whitespace, identifiers and punctuation.
    fn code(&mut self, start: usize, end: usize) {
        let mut i = start;
        while i < end {
            let b = self.bytes[i];
            if is_ident_byte(b) {
                let stop = (i..end)
                    .find(|&j| !is_ident_byte(self.bytes[j]))
                    .unwrap_or(end);
                self.push(Kind::Ident, i, stop);
                i = stop;
                continue;
            }
            match b {
                b'\n' => self.new_line = true,
                b' ' | b'\t' | b'\r' | 0x0b | 0x0c => {}
                _ => self.push(Kind::Punct(b), i, i + 1),
            }
            i += 1;
        }
    }

    /// A run of non-code bytes: leading comments are dropped, the rest is one opaque token.
    fn literal(&mut self, start: usize, end: usize) {
        let mut i = start;
        while i < end {
            let rest = &self.bytes[i..end];
            if rest.starts_with(b"//") {
                return;
            }
            if !rest.starts_with(b"/*") {
                self.push(Kind::Opaque, i, end);
                return;
            }
            let close = rest
                .windows(2)
                .skip(2)
                .position(|w| w == b"*/")
                .map_or(rest.len(), |n| n + 4);
            if rest[..close].contains(&b'\n') {
                self.new_line = true;
            }
            i += close;
        }
    }
}

/// End of the run of bytes starting at `start` whose code flag equals `in_code[start]`.
fn run_end(in_code: &[bool], start: usize) -> usize {
    let flag = in_code[start];
    in_code[start..]
        .iter()
        .position(|&c| c != flag)
        .map_or(in_code.len(), |n| start + n)
}

/// The tokens of `text`, with brackets paired.
pub(super) fn tokens(text: &str) -> Vec<Token> {
    let scan = scan_code(text);
    let mut lexer = Lexer {
        bytes: text.as_bytes(),
        tokens: Vec::new(),
        new_line: true,
    };
    let mut i = 0;
    while i < scan.in_code.len() {
        let end = run_end(&scan.in_code, i);
        if scan.in_code[i] {
            lexer.code(i, end);
        } else {
            lexer.literal(i, end);
        }
        i = end;
    }
    pair_brackets(lexer.tokens)
}

fn opener_of(closer: u8) -> Option<u8> {
    match closer {
        b')' => Some(b'('),
        b']' => Some(b'['),
        b'}' => Some(b'{'),
        _ => None,
    }
}

/// Pairs each closing bracket with the open one on top of the stack when they match; a closer
/// that does not match is left unpaired, as are openers never closed.
fn pair_brackets(tokens: Vec<Token>) -> Vec<Token> {
    let mut open: Vec<usize> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let Kind::Punct(b) = token.kind else {
            continue;
        };
        match (b, opener_of(b)) {
            (b'(' | b'[' | b'{', _) => open.push(index),
            (_, Some(opener)) => {
                let top = open.last().map(|&o| tokens[o].kind);
                if top == Some(Kind::Punct(opener)) {
                    pairs.extend(open.pop().map(|o| (o, index)));
                }
            }
            _ => {}
        }
    }
    let mut partner = vec![None; tokens.len()];
    for (o, c) in pairs {
        partner[o] = Some(c);
        partner[c] = Some(o);
    }
    tokens
        .into_iter()
        .zip(partner)
        .map(|(token, partner)| Token { partner, ..token })
        .collect()
}
