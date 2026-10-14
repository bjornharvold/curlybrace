//! Text sanitising for exports: one-line comments, JavaScript string literals, CSV cells.
//!
//! These are the only gates between free text (labels, names, values) and the files people run
//! or open in a spreadsheet, so each is small and exhaustively tested with hostile input.

/// Longest comment text, in characters.
pub const MAX_COMMENT_CHARS: usize = 200;

/// Characters that end a line in JavaScript or reorder displayed text (Trojan Source), besides
/// the control characters.
fn is_line_or_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{2028}' | '\u{2029}' | '\u{061c}' | '\u{200e}' | '\u{200f}'
            | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn is_unsafe_in_text(c: char) -> bool {
    c.is_control() || is_line_or_bidi_control(c)
}

/// `s` as ONE safe comment line: control characters, line and paragraph separators and
/// bidirectional overrides become spaces, runs of whitespace collapse to one space, the ends are
/// trimmed and the result is cut to [`MAX_COMMENT_CHARS`] characters.
pub fn comment_text(s: &str) -> String {
    let spaced: String = s
        .chars()
        .map(|c| if is_unsafe_in_text(c) { ' ' } else { c })
        .collect();
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let capped: String = collapsed.chars().take(MAX_COMMENT_CHARS).collect();
    capped.trim_end().to_string()
}

fn push_escaped(out: &mut String, c: char) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        c if is_unsafe_in_text(c) => out.push_str(&format!("\\u{:04x}", u32::from(c))),
        c => out.push(c),
    }
}

/// `s` as a JavaScript string literal: JSON escaping, plus the control characters, U+2028/2029
/// (line terminators in JavaScript) and bidirectional overrides as `\uXXXX`. The output is one
/// line of printable characters, so it can never end the statement it sits in.
pub fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        push_escaped(&mut out, c);
    }
    out.push('"');
    out
}

fn is_formula_trigger(c: char) -> bool {
    matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r')
}

/// `s` as one CSV cell (RFC 4180 quoting; `;` is quoted too, because spreadsheets in
/// semicolon locales split on it). A cell a spreadsheet would read as a formula (starting, after
/// optional blanks, with `=`, `+`, `-`, `@`, tab or CR) gets a leading `'`.
pub fn csv_cell(s: &str) -> String {
    // Spreadsheets skip leading blanks before they decide a cell is a formula.
    let formula = s.chars().next().is_some_and(is_formula_trigger)
        || s.trim_start()
            .chars()
            .next()
            .is_some_and(is_formula_trigger);
    let guarded = if formula {
        format!("'{s}")
    } else {
        s.to_string()
    };
    if guarded.contains([',', ';', '"', '\r', '\n']) {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

#[cfg(test)]
#[path = "sanitize_tests.rs"]
mod tests;
