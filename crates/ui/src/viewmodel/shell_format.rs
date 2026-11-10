//! The Shell tab's "Format" action: re-indents a mongosh (JavaScript) script by bracket depth.
//! Pure and egui-free; no user-facing strings.
//!
//! One left-to-right pass tracks the lexical state (code, `'`/`"` strings, template literals with
//! `${ ... }` nesting, line and block comments) and the open `{ [ (`. A line is indented by two
//! spaces per level, where a level is a line that left a bracket open (so `f({` opens one level,
//! not two). A bracket's body sits one level below the line that opened it; a line that starts
//! with closing brackets sits at the level of the line that opened the first of them (so
//! `}, {` re-opens at the level of its opener). Trailing whitespace is trimmed and blank lines
//! stay blank. Lines that start inside a string, template literal or block comment are text of
//! that literal, so they are left exactly as they are. Line endings (LF or CRLF) are never
//! touched and long lines are not reflowed.
//!
//! Only brackets drive the indent: leading-`.` chains and braceless continuations (`if (x)`
//! followed by a bare statement) are flattened to the bracket level.
//!
//! A `/` that follows nothing, or one of `( , = : [ { ! & | ? ;  +` on its line, and has a closing
//! `/` on the same line, is read as a regex literal and skipped. Anything else (including
//! `return /re/` and `) /re/`) is read as division, so a quote or bracket in such a regex can
//! still skew the depth or the literal state.
use std::ops::Range;

/// Spaces per nesting level.
const INDENT_WIDTH: usize = 2;
/// Levels beyond this are not indented further, so a pathological script cannot blow up the text.
const MAX_LEVELS: usize = 32;

/// The formatted script and the selection carried over to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted {
    pub text: String,
    /// The same logical span as the input selection, as byte offsets into `text`.
    pub selection: Option<Range<usize>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lex {
    Code,
    Single,
    Double,
    Template,
    LineComment,
    BlockComment,
}

/// What the scan learned about one line (without its `\n` / `\r\n`).
struct Line {
    start: usize,
    content_end: usize,
    end: usize,
    /// Indent levels, with the brackets the line starts by closing already closed.
    level: usize,
    /// The line begins in code, so its indentation is ours to change.
    in_code: bool,
    /// Trailing whitespace is not part of a literal.
    trim: bool,
}

/// Where one line's text went, for carrying offsets over.
struct Edit {
    old_start: usize,
    old_content_end: usize,
    new_start: usize,
    new_content_end: usize,
    old_lead: usize,
    new_lead: usize,
}

/// Formats `text`; with a selection only the lines it touches change. Offsets that are not on
/// char boundaries are widened to the enclosing ones.
pub fn format_script(text: &str, selection: Option<Range<usize>>) -> Formatted {
    let lines = scan(text);
    let range = selection.map(|sel| clamp(text, sel));
    let touched = range.as_ref().map(|r| touched_lines(&lines, r));

    let mut out = String::with_capacity(text.len());
    let mut edits = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let selected = touched.as_ref().is_none_or(|t| t.contains(&i));
        let content = &text[line.start..line.content_end];
        let new_start = out.len();
        let (old_lead, new_lead) = if selected && line.in_code {
            let body = content.trim_start_matches([' ', '\t']);
            let body = if line.trim {
                body.trim_end_matches([' ', '\t'])
            } else {
                body
            };
            let indent = if body.is_empty() {
                0
            } else {
                line.level.min(MAX_LEVELS) * INDENT_WIDTH
            };
            out.extend(std::iter::repeat_n(' ', indent));
            out.push_str(body);
            (
                content.len() - content.trim_start_matches([' ', '\t']).len(),
                indent,
            )
        } else {
            out.push_str(content);
            (0, 0)
        };
        edits.push(Edit {
            old_start: line.start,
            old_content_end: line.content_end,
            new_start,
            new_content_end: out.len(),
            old_lead,
            new_lead,
        });
        out.push_str(&text[line.content_end..line.end]);
    }
    let selection = range.map(|r| map_offset(&edits, r.start)..map_offset(&edits, r.end));
    Formatted {
        text: out,
        selection,
    }
}

/// Start floors and end ceils to a char boundary, so a split character is wholly inside.
fn clamp(text: &str, sel: Range<usize>) -> Range<usize> {
    let (a, b) = if sel.start <= sel.end {
        (sel.start, sel.end)
    } else {
        (sel.end, sel.start)
    };
    let mut start = a.min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = b.min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    start..end
}

/// Indexes of the lines the range touches. A range that ends right at a line start does not
/// touch that line; a cursor (empty range) touches its own line.
fn touched_lines(lines: &[Line], range: &Range<usize>) -> Range<usize> {
    let line_at = |p: usize| lines.partition_point(|l| l.start <= p).saturating_sub(1);
    let first = line_at(range.start);
    let mut last = line_at(range.end);
    if range.end > range.start && last > first && lines[last].start == range.end {
        last -= 1;
    }
    first..last + 1
}

fn map_offset(edits: &[Edit], p: usize) -> usize {
    let i = edits
        .partition_point(|e| e.old_start <= p)
        .saturating_sub(1);
    let e = &edits[i];
    let col = p - e.old_start;
    if p >= e.old_content_end {
        e.new_content_end + (p - e.old_content_end)
    } else if col == e.old_lead {
        // The first character of the text stays the first character of the text.
        e.new_start + e.new_lead
    } else if col < e.old_lead {
        e.new_start + col.min(e.new_lead)
    } else {
        (e.new_start + e.new_lead + (col - e.old_lead)).min(e.new_content_end)
    }
}

/// The scanner's running state across lines.
struct Scanner {
    state: Lex,
    /// Open brackets, each as the indent level of its body: one more than the level of the line
    /// that opened it.
    open: Vec<usize>,
    /// `open.len()` where each unfinished `${` was entered; its closing `}` resumes the template.
    subst: Vec<usize>,
    /// Indent level of the line being scanned.
    level: usize,
}

impl Scanner {
    fn push(&mut self) {
        self.open.push(self.level + 1);
    }

    fn pop(&mut self) {
        self.open.pop();
    }

    /// Level of a line that starts with `closers` closing brackets: the level of the line that
    /// opened the first of them, or the body level of the innermost open bracket.
    fn level_at_line_start(&self, closers: usize) -> usize {
        let body = self.open.last().copied().unwrap_or(0);
        if closers > 0 {
            body.saturating_sub(1)
        } else {
            body
        }
    }
}

fn is_closer(b: u8) -> bool {
    matches!(b, b'}' | b']' | b')')
}

/// One pass over the whole text. Delimiters are ASCII, so working on bytes never splits a char.
fn scan(text: &str) -> Vec<Line> {
    let bytes = text.as_bytes();
    let mut lines: Vec<Line> = Vec::new();
    let mut sc = Scanner {
        state: Lex::Code,
        open: Vec::new(),
        subst: Vec::new(),
        level: 0,
    };
    let mut start = 0;
    loop {
        let (content_end, end) = match bytes[start..].iter().position(|&b| b == b'\n') {
            Some(n) => {
                let nl = start + n;
                let ce = if nl > start && bytes[nl - 1] == b'\r' {
                    nl - 1
                } else {
                    nl
                };
                (ce, nl + 1)
            }
            None => (bytes.len(), bytes.len()),
        };
        let content = &bytes[start..content_end];
        let in_code = sc.state == Lex::Code;
        let closers = if in_code { leading_closers(content) } else { 0 };
        sc.level = sc.level_at_line_start(closers);
        let level = sc.level;
        let escaped = scan_line(content, &mut sc);
        let trim = matches!(sc.state, Lex::Code | Lex::LineComment);
        lines.push(Line {
            start,
            content_end,
            end,
            level,
            in_code,
            trim,
        });
        // A newline ends a line comment, and an unterminated string unless the line ended in `\`.
        sc.state = match sc.state {
            Lex::LineComment => Lex::Code,
            Lex::Single | Lex::Double if !escaped => Lex::Code,
            s => s,
        };
        if end >= bytes.len() {
            break;
        }
        start = end;
    }
    lines
}

/// How many closing brackets the line starts with (blanks between them allowed).
fn leading_closers(line: &[u8]) -> usize {
    line.iter()
        .filter(|&&b| b != b' ' && b != b'\t')
        .take_while(|&&b| is_closer(b))
        .count()
}

/// Whether a `/` at `i` starts a regex literal rather than a division: nothing, or an operator
/// or opening punctuation, comes before it on the line.
fn starts_regex(line: &[u8], i: usize) -> bool {
    match line[..i].iter().rev().find(|&&b| b != b' ' && b != b'\t') {
        None => true,
        Some(b) => b"(,=:[{!&|?;+".contains(b),
    }
}

/// The index of the `/` that closes the regex literal opened at `i`, on the same line.
fn regex_end(line: &[u8], i: usize) -> Option<usize> {
    let mut class = false;
    let mut j = i + 1;
    while j < line.len() {
        match line[j] {
            b'\\' => j += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => return Some(j),
            _ => {}
        }
        j += 1;
    }
    None
}

/// Advances the lexical state over one line; true when it ends in a dangling `\`.
fn scan_line(line: &[u8], sc: &mut Scanner) -> bool {
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        let next = line.get(i + 1).copied();
        match sc.state {
            Lex::Code => match b {
                b'\'' => sc.state = Lex::Single,
                b'"' => sc.state = Lex::Double,
                b'`' => sc.state = Lex::Template,
                b'/' if next == Some(b'/') => {
                    sc.state = Lex::LineComment;
                    i += 1;
                }
                b'/' if next == Some(b'*') => {
                    sc.state = Lex::BlockComment;
                    i += 1;
                }
                b'/' if starts_regex(line, i) => {
                    if let Some(end) = regex_end(line, i) {
                        i = end;
                    }
                }
                b'{' | b'[' | b'(' => sc.push(),
                b'}' | b']' | b')' => {
                    sc.pop();
                    if b == b'}' && sc.subst.last() == Some(&sc.open.len()) {
                        sc.subst.pop();
                        sc.state = Lex::Template;
                    }
                }
                _ => {}
            },
            Lex::Single | Lex::Double | Lex::Template => {
                let quote = match sc.state {
                    Lex::Single => b'\'',
                    Lex::Double => b'"',
                    _ => b'`',
                };
                if b == b'\\' {
                    if i + 1 >= line.len() {
                        return true;
                    }
                    i += 1;
                } else if b == quote {
                    sc.state = Lex::Code;
                } else if sc.state == Lex::Template && b == b'$' && next == Some(b'{') {
                    sc.subst.push(sc.open.len());
                    sc.push();
                    sc.state = Lex::Code;
                    i += 1;
                }
            }
            Lex::LineComment => return false,
            Lex::BlockComment => {
                if b == b'*' && next == Some(b'/') {
                    sc.state = Lex::Code;
                    i += 1;
                }
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
#[path = "shell_format_tests.rs"]
mod tests;
