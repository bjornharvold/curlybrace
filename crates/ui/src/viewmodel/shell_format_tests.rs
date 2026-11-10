use super::*;

fn fmt(text: &str) -> String {
    format_script(text, None).text
}

fn fmt_sel(text: &str, sel: Range<usize>) -> Formatted {
    format_script(text, Some(sel))
}

#[test]
fn indents_nested_braces_brackets_and_parens_by_two_spaces() {
    let src = "db.a.aggregate([\n{ $match: {\nx: 1,\ny: [\n2,\nfoo(\n3\n)\n]\n} }\n])\n";
    let want = "db.a.aggregate([\n  { $match: {\n    x: 1,\n    y: [\n      2,\n      foo(\n        3\n      )\n    ]\n  } }\n])\n";
    assert_eq!(fmt(src), want);
}

#[test]
fn closing_bracket_line_dedents() {
    assert_eq!(fmt("f({\na: 1\n})\n"), "f({\n  a: 1\n})\n");
    assert_eq!(
        fmt("if (x) {\n      y();\n        }\n"),
        "if (x) {\n  y();\n}\n"
    );
}

#[test]
fn strings_template_literals_and_comments_are_untouched() {
    // Brackets inside literals and comments do not change the depth.
    let src = "a({\nb: '{[(',\nc: \"}])\",\nd: `{ ${ x } (`,\n// {{{\ne: 1 /* ) ) */\n})\nz()\n";
    let want = "a({\n  b: '{[(',\n  c: \"}])\",\n  d: `{ ${ x } (`,\n  // {{{\n  e: 1 /* ) ) */\n})\nz()\n";
    assert_eq!(fmt(src), want);

    // Lines inside a multi-line template or block comment keep their text exactly.
    let src = "f(\n`a\n     b  \n  ${\n1 }\n`,\n/* x\n      y */\n1)\n";
    let want = "f(\n  `a\n     b  \n  ${\n    1 }\n`,\n  /* x\n      y */\n  1)\n";
    assert_eq!(fmt(src), want);
}

#[test]
fn template_substitution_nesting_returns_to_the_template() {
    let src = "x = `a ${ {b: `c ${ d }`}.b } }`\n{\n1\n}\n";
    assert_eq!(fmt(src), "x = `a ${ {b: `c ${ d }`}.b } }`\n{\n  1\n}\n");
}

#[test]
fn selection_depth_comes_from_text_before_it() {
    let src = "a({\nb: [\nc,\nd\n]\n})\n";
    let start = src.find("c,").unwrap();
    let end = src.find("d\n").unwrap() + 1;
    let got = fmt_sel(src, start..end);
    assert_eq!(got.text, "a({\nb: [\n    c,\n    d\n]\n})\n");
}

#[test]
fn only_selected_lines_change() {
    let src = "  a(\n b,\n   c ,  \n d\n )\n";
    let start = src.find("b,").unwrap();
    let end = src.find("c ,").unwrap() + 1;
    let got = fmt_sel(src, start..end);
    assert_eq!(got.text, "  a(\n  b,\n  c ,\n d\n )\n");

    // A selection that ends at the start of the next line does not touch that line.
    let start = src.find("b,").unwrap();
    let end = src.find("   c").unwrap();
    assert_eq!(
        fmt_sel(src, start..end).text,
        "  a(\n  b,\n   c ,  \n d\n )\n"
    );

    // A cursor touches its own line only.
    let at = src.find(" d").unwrap() + 1;
    assert_eq!(fmt_sel(src, at..at).text, "  a(\n b,\n   c ,  \n  d\n )\n");
}

#[test]
fn trailing_whitespace_is_trimmed() {
    assert_eq!(fmt("a;  \t\n  \n\t\nb;   "), "a;\n\n\nb;");
    // Inside a template literal it is part of the text, and so is a line that starts in one.
    assert_eq!(fmt("`a  \nb`  \n"), "`a  \nb`  \n");
    assert_eq!(fmt("x = `a  \n`;  \ny  \n"), "x = `a  \n`;  \ny\n");
    assert_eq!(fmt("// note   \nx\n"), "// note\nx\n");
}

#[test]
fn unbalanced_input_never_panics_or_goes_negative() {
    assert_eq!(fmt("}\n)\n]\nx\n"), "}\n)\n]\nx\n");
    assert_eq!(fmt("}}}\n{\na\n"), "}}}\n{\n  a\n");
    let _ = fmt("((((\n`${ {\n'\n\"\\\n/*\n");
    let _ = fmt("");
    let _ = fmt("\n\n\r\n");
    let deep = "{\n".repeat(500) + "x\n";
    let out = fmt(&deep);
    assert!(out.ends_with(&format!("{}x\n", " ".repeat(MAX_LEVELS * INDENT_WIDTH))));
}

#[test]
fn crlf_is_preserved() {
    assert_eq!(fmt("a({\r\nb:1  \r\n})\r\n"), "a({\r\n  b:1\r\n})\r\n");
    assert_eq!(fmt("a({\r\n\r\nb\r\n})"), "a({\r\n\r\n  b\r\n})");
    // A backslash before CRLF continues a string.
    assert_eq!(fmt("'a\\\r\n  b'\r\nx"), "'a\\\r\n  b'\r\nx");
}

#[test]
fn selection_maps_to_the_new_text() {
    let src = "f({\n      a: 1,\n   b: 2\n})\n";
    let start = src.find("a: 1").unwrap();
    let end = src.find(" 2").unwrap() + 2;
    let got = fmt_sel(src, start..end);
    assert_eq!(got.text, "f({\n  a: 1,\n  b: 2\n})\n");
    let sel = got.selection.unwrap();
    assert_eq!(&got.text[sel], "a: 1,\n  b: 2");

    // A selection that begins at a line start keeps the new indent inside it.
    let start = src.find("      a").unwrap();
    let got = fmt_sel(src, start..src.len() - 1);
    assert_eq!(got.text, "f({\n  a: 1,\n  b: 2\n})\n");
    let sel = got.selection.unwrap();
    assert_eq!(&got.text[sel], "  a: 1,\n  b: 2\n})");

    // Formatting everything leaves no selection when none was given.
    assert_eq!(format_script(src, None).selection, None);
}

#[test]
fn selection_off_char_boundaries_is_widened_not_a_panic() {
    let src = "f({\n  a: 'é😀',\n})\n";
    let mid_e = src.find('é').unwrap() + 1;
    let mid_emoji = src.find('😀').unwrap() + 2;
    let got = fmt_sel(src, mid_e..mid_emoji);
    let sel = got.selection.unwrap();
    assert_eq!(&got.text[sel], "é😀");

    // Past the end and reversed ranges are clamped.
    let got = fmt_sel(
        src,
        Range {
            start: 10_000,
            end: 3,
        },
    );
    assert!(got.selection.is_some());
    assert_eq!(got.text, "f({\n  a: 'é😀',\n})\n");
}

#[test]
fn multibyte_text_in_strings_and_comments_is_kept() {
    let src = "f({\na: 'é😀{',\n// ü }\nb: `日本\n語`,\n})\n";
    let want = "f({\n  a: 'é😀{',\n  // ü }\n  b: `日本\n語`,\n})\n";
    assert_eq!(fmt(src), want);
}

const CORPUS: &[&str] = &[
    "",
    "\n",
    "db.users.find({ a: 1 })\n",
    "const x = `a ${ b ? `c ${ d }` : '}' } e`;\nx\n",
    "/* open\n  {\n*/ f(\n1)\n",
    "// ' \" ` /*\nf('it\\'s', \"q\\\"\", `t\\``)\n",
    "a = '\n{\nb = \"\n}\n",
    "`${`${`${\n}`}`}`\n",
    "f(\r\n  {\r\n\ta: [1,\r\n2]\r\n  }\r\n)\r\n",
    "'x\\\n   y'\n}}}{{{\n",
    "  é😀 {\n  日本 }\n",
    "\\\n\\\r\n'\\",
    "$ { } ${ ${ } }\n",
];

const TOKENS: &[&str] = &[
    "{", "}", "(", ")", "[", "]", "'", "\"", "`", "//", "/*", "*/", "\n", "\n", "\r\n", " ", "  ",
    "\t", "a", "b1", "${", "}", "\\", "é", "😀", ",", ";", "/",
];

/// Tiny seeded LCG so a failing case reproduces from its printed seed and index.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

fn non_ws(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn check_script(label: &str, src: &str) {
    let once = format_script(src, None).text;
    let twice = format_script(&once, None).text;
    assert_eq!(once, twice, "not idempotent: {label}: {src:?}");
    assert_eq!(
        non_ws(src),
        non_ws(&once),
        "non-whitespace changed: {label}: {src:?}"
    );
    // Formatting any char-aligned slice of lines must not panic either.
    let len = src.len();
    for (a, b) in [
        (0, len),
        (len / 3, len / 2),
        (len / 2, len / 3),
        (len, len + 5),
    ] {
        let got = format_script(src, Some(a..b));
        let sel = got.selection.expect("selection is carried over");
        assert!(got.text.is_char_boundary(sel.start) && got.text.is_char_boundary(sel.end));
        assert!(
            sel.start <= sel.end && sel.end <= got.text.len(),
            "{label}: {src:?}"
        );
        assert_eq!(non_ws(src), non_ws(&got.text), "{label}: {src:?}");
    }
}

#[test]
fn is_idempotent() {
    const SEED: u64 = 0x5EED_CAFE;
    for (i, src) in CORPUS.iter().enumerate() {
        check_script(&format!("corpus #{i}"), src);
    }
    let mut rng = Lcg(SEED);
    for case in 0..400 {
        let n = 1 + rng.next(40);
        let src: String = (0..n).map(|_| TOKENS[rng.next(TOKENS.len())]).collect();
        check_script(&format!("seed {SEED:#x} case {case}"), &src);
    }
}
