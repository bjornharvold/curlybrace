//! The Shell tab's "Format" action: re-indents a mongosh (JavaScript) script by bracket depth.
//! Pure and egui-free; no user-facing strings.
//!
//! One left-to-right pass tracks the lexical state (code, `'`/`"` strings, template literals with
//! `${ ... }` nesting, line and block comments) and the open `{ [ (`. A line is indented by two
//! spaces per level, where a level is a line that left a bracket open (so `f({` opens one level,
//! not two); a line that starts with closing brackets is indented as if they were already closed.
//! Trailing whitespace is trimmed and blank lines stay blank. Lines that start inside a string,
//! template literal or block comment are text of that literal, so they are left exactly as they
//! are. Line endings (LF or CRLF) are never touched and long lines are not reflowed.
//!
//! Known limitation: regex literals are plain code to the scanner, so a quote or bracket inside
//! one (`/["{]/`) is read as a real quote or bracket and can skew the depth or the literal state.
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
    } else if col <= e.old_lead {
        e.new_start + col.min(e.new_lead)
    } else {
        (e.new_start + e.new_lead + (col - e.old_lead)).min(e.new_content_end)
    }
}

/// The scanner's running state across lines.
struct Scanner {
    state: Lex,
    /// Open brackets as (line it opened on, indent levels up to and including it).
    open: Vec<(usize, usize)>,
    /// `open.len()` where each unfinished `${` was entered; its closing `}` resumes the template.
    subst: Vec<usize>,
}

impl Scanner {
    /// Levels of the first `n` open brackets: one per distinct line that opened them.
    fn levels(&self, n: usize) -> usize {
        n.checked_sub(1).map_or(0, |i| self.open[i].1)
    }

    fn push(&mut self, line: usize) {
        let level = match self.open.last() {
            Some(&(l, lv)) if l == line => lv,
            Some(&(_, lv)) => lv + 1,
            None => 1,
        };
        self.open.push((line, level));
    }

    fn pop(&mut self) {
        self.open.pop();
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
        let level = if in_code {
            sc.levels(sc.open.len() - leading_closers(content, sc.open.len()))
        } else {
            0
        };
        let escaped = scan_line(content, lines.len(), &mut sc);
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

/// How many of the brackets open at the line's start it closes before anything else.
fn leading_closers(line: &[u8], open: usize) -> usize {
    let n = line
        .iter()
        .filter(|&&b| b != b' ' && b != b'\t')
        .take_while(|&&b| is_closer(b))
        .count();
    n.min(open)
}

/// Advances the lexical state over one line; true when it ends in a dangling `\`.
fn scan_line(line: &[u8], line_no: usize, sc: &mut Scanner) -> bool {
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
                b'{' | b'[' | b'(' => sc.push(line_no),
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
                    sc.push(line_no);
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
