//! Drift guard: UI colors and font sizes come from the theme (`crates/ui/src/theme/`), never from
//! literals in views or the app shell.
//!
//! Forbidden outside the theme: every `Color32` associated item except `Color32::TRANSPARENT`
//! (channel constructors such as `from_rgb`, `from_gray`, `from_rgba_*`, `from_hex`, and named
//! constants such as `WHITE` or `GRAY`), `hex_color!`, and literal font sizes (`.size(14.0)`,
//! `FontId::new(14.0, ..)`, `FontId::proportional(14.0)`, `FontId::monospace(14.0)`,
//! `size: 14.0`). Variables and constants as sizes are fine: the rule is about literals.
//!
//! Limits of the scan: comments and string/char literals are blanked before matching, so text
//! that merely mentions a constructor never trips it. Test code is skipped by file name
//! (`*_tests.rs`, `tests.rs`) and, in other files, from the first `#[cfg(test)]` line to the end
//! (production code is expected to precede its test module, as in `fonts_tests.rs`'s glyph scan).
//! The scan is textual: a constructor hidden behind a macro or a `use ...::Color32 as C` alias is
//! not seen.

use std::path::{Path, PathBuf};

/// Files that may build colors from channel values. Names files, not patterns.
const ALLOW_LIST: &[&str] = &["crates/ui/src/views/connection_badge.rs"];

const THEME_DIR: &str = "crates/ui/src/theme/";

/// Replaces comments, string literals and char literals with spaces, keeping newlines so
/// offsets still map to line numbers.
fn blank_comments_and_strings(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if c == 'r' && matches!(next, Some('"' | '#')) && is_raw_string_start(&chars, i) {
            let hashes = chars[i + 1..].iter().take_while(|&&h| h == '#').count();
            let end = raw_string_end(&chars, i + 2 + hashes, hashes);
            out.extend(chars[i..end].iter().map(|&b| blank(b)));
            i = end;
        } else if c == '"' {
            out.push(' ');
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    out.push(blank(chars[i]));
                    i += 1;
                }
                out.push(blank(chars[i]));
                i += 1;
            }
            out.push(' ');
            i += 1;
        } else if c == '\'' && char_literal_len(&chars, i).is_some() {
            let len = char_literal_len(&chars, i).unwrap_or(1);
            out.extend(std::iter::repeat_n(' ', len));
            i += len;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn is_raw_string_start(chars: &[char], i: usize) -> bool {
    let hashes = chars[i + 1..].iter().take_while(|&&h| h == '#').count();
    chars.get(i + 1 + hashes) == Some(&'"') && (i == 0 || !is_ident(chars[i - 1]))
}

fn raw_string_end(chars: &[char], mut i: usize, hashes: usize) -> usize {
    while i < chars.len() {
        if chars[i] == '"' && chars[i + 1..].iter().take_while(|&&h| h == '#').count() >= hashes {
            return i + 1 + hashes;
        }
        i += 1;
    }
    chars.len()
}

/// Length of the char literal starting at `i` (`'a'`, `'\n'`, `'\u{e000}'`), or `None` for a
/// lifetime such as `'a`.
fn char_literal_len(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i + 1) == Some(&'\\') {
        let close = chars[i + 2..].iter().position(|&c| c == '\'')?;
        return Some(close + 3);
    }
    (chars.get(i + 2) == Some(&'\'')).then_some(3)
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 1-based line numbers (with the trimmed original line) of every drift in `src`.
fn violations(src: &str) -> Vec<(usize, String)> {
    let production: String = src
        .lines()
        .take_while(|l| l.trim() != "#[cfg(test)]")
        .collect::<Vec<_>>()
        .join("\n");
    let code = blank_comments_and_strings(&production);
    let mut hits = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    offsets.extend(color_offsets(&code));
    offsets.extend(literal_size_offsets(&code));
    offsets.sort_unstable();
    offsets.dedup();
    let original: Vec<&str> = production.lines().collect();
    for off in offsets {
        let line = code[..off].matches('\n').count();
        hits.push((line + 1, original[line].trim().to_owned()));
    }
    hits.dedup();
    hits
}

fn color_offsets(code: &str) -> Vec<usize> {
    let mut found: Vec<usize> = code
        .match_indices("Color32::")
        .filter(|(at, m)| !code[at + m.len()..].starts_with("TRANSPARENT"))
        .map(|(at, _)| at)
        .collect();
    found.extend(code.match_indices("hex_color!").map(|(at, _)| at));
    found
}

fn literal_size_offsets(code: &str) -> Vec<usize> {
    const OPENERS: &[&str] = &[
        ".size(",
        "FontId::new(",
        "FontId::proportional(",
        "FontId::monospace(",
    ];
    let starts_with_digit =
        |rest: &str| rest.trim_start().starts_with(|c: char| c.is_ascii_digit());
    let mut found = Vec::new();
    for opener in OPENERS {
        for (at, m) in code.match_indices(opener) {
            if starts_with_digit(&code[at + m.len()..]) {
                found.push(at);
            }
        }
    }
    for (at, m) in code.match_indices("size:") {
        let boundary = code[..at].chars().next_back().is_none_or(|c| !is_ident(c));
        if boundary && starts_with_digit(&code[at + m.len()..]) {
            found.push(at);
        }
    }
    found
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// Scanned files as workspace-relative, forward-slash paths.
fn scanned_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    walk(&root.join("crates/ui/src"), &mut files);
    walk(&root.join("crates/app/src"), &mut files);
    let mut rel: Vec<String> = files
        .iter()
        .map(|f| {
            let p = f.canonicalize().expect("source path");
            p.strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|p| !p.starts_with(THEME_DIR))
        .filter(|p| {
            let name = p.rsplit('/').next().unwrap_or(p);
            name != "tests.rs" && !name.ends_with("_tests.rs")
        })
        .collect();
    rel.sort();
    rel
}

#[test]
fn theme_drift() {
    let root = workspace_root();
    let mut report = Vec::new();
    for file in scanned_files(&root) {
        if ALLOW_LIST.contains(&file.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(root.join(&file)).expect("utf-8 source");
        for (line, text) in violations(&text) {
            report.push(format!("{file}:{line}: {text}"));
        }
    }
    assert!(
        report.is_empty(),
        "hard-coded colors or font sizes outside the theme (use roles/text styles from \
         crates/ui/src/theme/):\n{}",
        report.join("\n")
    );
}

#[test]
fn theme_drift_allow_list_is_one_file() {
    assert_eq!(
        ALLOW_LIST.len(),
        1,
        "the allow-list is the tag color file only"
    );
    let path = workspace_root().join(ALLOW_LIST[0]);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("allow-listed file {path:?} is gone: {e}"));
    assert!(
        text.contains("Color32::from_rgb"),
        "{} no longer builds a color from channels; drop it from the allow-list",
        ALLOW_LIST[0]
    );
}

#[test]
fn matcher_ignores_comments_and_strings() {
    let src = "// Color32::from_rgb(1, 2, 3)\nlet s = \"Color32::WHITE .size(14.0)\";\n\
               /* FontId::new(12.0, f) */\nlet r = r#\"Color32::from_gray(3)\"#;\nlet c = '\"';";
    assert_eq!(violations(src), vec![]);
}

#[test]
fn matcher_catches_color_constructors_and_constants() {
    for code in [
        "let c = Color32::from_rgb(1, 2, 3);",
        "let c = Color32::from_gray(9);",
        "let c = Color32::from_rgba_unmultiplied(1, 2, 3, 4);",
        "let c = Color32::from_black_alpha(9);",
        "let c = Color32::WHITE;",
        "let c = hex_color!(\"#fff\");",
    ] {
        assert_eq!(violations(code).len(), 1, "{code}");
    }
}

#[test]
fn matcher_allows_transparent_and_theme_values() {
    let src = "let c = Color32::TRANSPARENT;\nlet f = FontId::proportional(tokens::BODY);\n\
               let t = RichText::new(x).size(font_size);\nlet s = Spinner::new().size(SIZE);";
    assert_eq!(violations(src), vec![]);
}

#[test]
fn matcher_catches_literal_font_sizes() {
    for code in [
        "RichText::new(x).size(14.0)",
        "RichText::new(x).size(\n    14.0)",
        "FontId::new(14.0, FontFamily::Proportional)",
        "FontId::proportional(12.0)",
        "FontId::monospace(12.0)",
        "FontId { size: 12.0, family }",
    ] {
        assert_eq!(violations(code).len(), 1, "{code}");
    }
}

#[test]
fn matcher_reports_line_numbers_and_stops_at_cfg_test() {
    let src = "fn a() {}\nlet c = Color32::RED;\n#[cfg(test)]\nmod t { let c = Color32::BLUE; }";
    assert_eq!(
        violations(src),
        vec![(2, "let c = Color32::RED;".to_owned())]
    );
}
