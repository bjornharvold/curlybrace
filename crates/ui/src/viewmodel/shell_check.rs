//! The Shell tab's pre-run syntax check: the first unclosed or stray bracket, or unterminated
//! string, template literal or block comment in a mongosh script. Pure and egui-free; the
//! problems are enum values, the view turns them into translated messages.
use super::shell_format::{Literal, scan_code};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProblemKind {
    UnclosedBracket(char),
    UnexpectedClosing(char),
    MismatchedClosing { opener: char, found: char },
    UnterminatedString,
    UnterminatedTemplate,
    UnterminatedComment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxProblem {
    pub kind: ProblemKind,
    /// Bytes of the offending bracket, or of the opening delimiter of the unterminated literal.
    pub range: Range<usize>,
    /// 1-based.
    pub line: usize,
    /// 1-based, in chars.
    pub column: usize,
}

/// The first problem in text order, or `None` when the brackets balance and every literal ends.
pub fn check(text: &str) -> Option<SyntaxProblem> {
    let scan = scan_code(text);
    let mut candidates: Vec<(ProblemKind, Range<usize>)> = scan
        .unterminated
        .iter()
        .map(|&(literal, at)| literal_problem(literal, at))
        .collect();
    let mut open: Vec<(char, usize)> = Vec::new();
    for (i, &b) in text.as_bytes().iter().enumerate() {
        if !scan.in_code[i] {
            continue;
        }
        match b {
            b'{' | b'[' | b'(' => open.push((b as char, i)),
            b'}' | b']' | b')' => match open.pop() {
                None => {
                    candidates.push((ProblemKind::UnexpectedClosing(b as char), i..i + 1));
                    break;
                }
                Some((opener, _)) if closer_of(opener) != b as char => {
                    let kind = ProblemKind::MismatchedClosing {
                        opener,
                        found: b as char,
                    };
                    candidates.push((kind, i..i + 1));
                    break;
                }
                Some(_) => {}
            },
            _ => {}
        }
    }
    // Brackets left open count only when the scan ran to the end; the innermost, nearest the end, is the one to fix.
    if !candidates.iter().any(|(k, _)| is_bracket_error(k))
        && let Some(&(opener, at)) = open.last()
    {
        candidates.push((ProblemKind::UnclosedBracket(opener), at..at + 1));
    }
    let (kind, range) = candidates.into_iter().min_by_key(|(_, r)| r.start)?;
    let (line, column) = position(text, range.start);
    Some(SyntaxProblem {
        kind,
        range,
        line,
        column,
    })
}

fn closer_of(opener: char) -> char {
    match opener {
        '{' => '}',
        '[' => ']',
        _ => ')',
    }
}

fn is_bracket_error(kind: &ProblemKind) -> bool {
    matches!(
        kind,
        ProblemKind::UnexpectedClosing(_) | ProblemKind::MismatchedClosing { .. }
    )
}

fn literal_problem(literal: Literal, at: usize) -> (ProblemKind, Range<usize>) {
    match literal {
        Literal::String => (ProblemKind::UnterminatedString, at..at + 1),
        Literal::Template => (ProblemKind::UnterminatedTemplate, at..at + 1),
        Literal::Comment => (ProblemKind::UnterminatedComment, at..at + 2),
    }
}

/// 1-based line and char column of the byte offset `at`.
fn position(text: &str, at: usize) -> (usize, usize) {
    let before = &text[..at];
    let line = 1 + before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |n| n + 1);
    (line, before[line_start..].chars().count() + 1)
}

#[cfg(test)]
#[path = "shell_check_tests.rs"]
mod tests;
