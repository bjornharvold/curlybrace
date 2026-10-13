use super::*;

fn one_safe_line(s: &str) -> bool {
    !s.chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202e}' | '\u{2066}'))
}

#[test]
fn comment_text_keeps_plain_text() {
    assert_eq!(comment_text("Nightly sync"), "Nightly sync");
    assert_eq!(comment_text(""), "");
}

#[test]
fn comment_text_collapses_every_line_break_and_control_char() {
    let hostile = "a\r\nb\nc\rd\te\u{0}f\u{2028}g\u{2029}h\u{85}i\u{7f}j";
    let out = comment_text(hostile);
    assert!(one_safe_line(&out), "{out:?}");
    assert_eq!(out, "a b c d e f g h i j");
}

#[test]
fn comment_text_cannot_start_a_new_script_line() {
    let out = comment_text("x\n); db.dropDatabase(); //");
    assert!(!out.contains('\n'));
    assert_eq!(out, "x ); db.dropDatabase(); //");
}

#[test]
fn comment_text_removes_bidirectional_overrides() {
    let out = comment_text("safe\u{202e}evil\u{2066}x\u{200f}y");
    assert!(one_safe_line(&out), "{out:?}");
    assert!(!out.contains('\u{200f}'));
}

#[test]
fn comment_text_trims_and_collapses_runs_of_space() {
    assert_eq!(comment_text("  a   \t\t b  \n "), "a b");
}

#[test]
fn comment_text_caps_length_in_chars_not_bytes() {
    let long = "\u{1f600}".repeat(500);
    let out = comment_text(&long);
    assert_eq!(out.chars().count(), MAX_COMMENT_CHARS);
    assert!(out.ends_with('\u{1f600}') || out.ends_with("..."));
    let ascii = "a".repeat(10_000);
    assert_eq!(comment_text(&ascii).chars().count(), MAX_COMMENT_CHARS);
}

#[test]
fn comment_text_cap_never_leaves_trailing_space() {
    let s = format!("{} b", "a".repeat(MAX_COMMENT_CHARS - 1));
    let out = comment_text(&s);
    assert!(!out.ends_with(' '));
    assert!(out.chars().count() <= MAX_COMMENT_CHARS);
}

fn round_trip(s: &str) -> String {
    let literal = js_string(s);
    assert!(
        literal.starts_with('"') && literal.ends_with('"'),
        "{literal}"
    );
    serde_json::from_str::<String>(&literal).expect("a js_string literal is a JSON string")
}

#[test]
fn js_string_round_trips_hostile_text() {
    for s in [
        "",
        "plain",
        "quote\" and \\ backslash",
        "line\nfeed\r\ncarriage\ttab",
        "ls\u{2028}ps\u{2029}",
        "nul\u{0}byte",
        "emoji \u{1f600} and \u{1f1e9}\u{1f1ea}",
        "*/ closes a comment",
        "</script><script>alert(1)</script>",
        "x\"); db.dropDatabase(); //",
        "bidi \u{202e}override",
        "\u{1}\u{1f}\u{7f}\u{85}",
        "back\u{8}form\u{c}feed",
    ] {
        assert_eq!(round_trip(s), s);
    }
}

#[test]
fn js_string_output_is_one_line_of_safe_characters() {
    let out = js_string("a\nb\u{2028}c\u{2029}d\u{0}e\u{202e}f\u{85}");
    assert!(one_safe_line(&out), "{out:?}");
    assert!(out.contains("\\u2028") && out.contains("\\u2029") && out.contains("\\u0000"));
    assert!(out.contains("\\n"));
}

#[test]
fn js_string_exact_forms() {
    assert_eq!(js_string("a\"b"), r#""a\"b""#);
    assert_eq!(js_string("a\\b"), r#""a\\b""#);
    assert_eq!(js_string("\u{0}"), r#""\u0000""#);
    assert_eq!(js_string("\u{1f600}"), "\"\u{1f600}\"");
}

#[test]
fn csv_cell_leaves_plain_text_alone() {
    assert_eq!(csv_cell("plain"), "plain");
    assert_eq!(csv_cell("unicod\u{e9} \u{1f600}"), "unicod\u{e9} \u{1f600}");
}

#[test]
fn csv_cell_empty_string_is_empty() {
    assert_eq!(csv_cell(""), "");
}

#[test]
fn csv_cell_quotes_commas_quotes_and_line_breaks() {
    assert_eq!(csv_cell("a,b"), "\"a,b\"");
    assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(csv_cell("two\nlines"), "\"two\nlines\"");
    assert_eq!(csv_cell("a\r\nb"), "\"a\r\nb\"");
}

#[test]
fn csv_cell_prefixes_every_formula_trigger() {
    assert_eq!(csv_cell("=1+1"), "'=1+1");
    assert_eq!(csv_cell("+1"), "'+1");
    assert_eq!(csv_cell("-1"), "'-1");
    assert_eq!(csv_cell("@SUM(A1)"), "'@SUM(A1)");
    assert_eq!(csv_cell("\tcmd"), "'\tcmd");
    assert_eq!(csv_cell("\rcmd"), "\"'\rcmd\"");
}

#[test]
fn csv_cell_prefix_and_quoting_combine() {
    assert_eq!(csv_cell("=A1,B1"), "\"'=A1,B1\"");
    assert_eq!(csv_cell("=\"x\""), "\"'=\"\"x\"\"\"");
}

#[test]
fn csv_cell_never_double_prefixes() {
    assert_eq!(csv_cell("'=1"), "'=1");
    assert_eq!(csv_cell(&csv_cell("=1")), "'=1");
}

#[test]
fn csv_cell_triggers_only_count_at_the_start() {
    assert_eq!(csv_cell("a=b"), "a=b");
    assert_eq!(csv_cell("a-b"), "a-b");
}
