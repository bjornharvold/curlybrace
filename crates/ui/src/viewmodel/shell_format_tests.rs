use super::*;

/// The text with every byte that is not code replaced by `.`.
fn code_of(text: &str) -> String {
    let scan = scan_code(text);
    text.bytes()
        .zip(&scan.in_code)
        .map(|(b, &c)| if c { b as char } else { '.' })
        .collect()
}

#[test]
fn scan_code_masks_strings_comments_template_text_and_regex_literals() {
    assert_eq!(code_of("a('x') // c"), "a(...) ....");
    assert_eq!(code_of("f(/[(]/)"), "f(.....)");
    assert_eq!(code_of("a /* ( */ b"), "a ....... b");
    // The `${ ... }` placeholder is code; its `$`, the backticks and the text are not.
    assert_eq!(code_of("`a${b}c`"), "...{b}..");
}

#[test]
fn scan_code_reports_unterminated_literals_with_their_start() {
    assert_eq!(
        scan_code("x = 'abc\ny").unterminated,
        [(Literal::String, 4)]
    );
    assert_eq!(scan_code("`a${b}").unterminated, [(Literal::Template, 0)]);
    assert_eq!(scan_code("a /* x").unterminated, [(Literal::Comment, 2)]);
    assert!(scan_code("'a\\\nb' + \"c\"").unterminated.is_empty());
}

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

#[test]
fn a_line_that_closes_and_reopens_sits_at_its_openers_level() {
    let cases = [
        (
            "db.c.updateMany({\n a: 1\n}, {\n $set: {b:2}\n})\n",
            "db.c.updateMany({\n  a: 1\n}, {\n  $set: {b:2}\n})\n",
        ),
        (
            "foo(function () {\nbar();\n}, 100)\n",
            "foo(function () {\n  bar();\n}, 100)\n",
        ),
        (
            "updateOne(filter, {\n$set: 1\n}, { upsert: true })\n",
            "updateOne(filter, {\n  $set: 1\n}, { upsert: true })\n",
        ),
        (
            "p.then(function () {\nx();\n}).then(function () {\ny();\n})\n",
            "p.then(function () {\n  x();\n}).then(function () {\n  y();\n})\n",
        ),
        (
            "if (a) {\nb();\n} else {\nc();\n}\n",
            "if (a) {\n  b();\n} else {\n  c();\n}\n",
        ),
        ("f([{\na: 1\n}])\n", "f([{\n  a: 1\n}])\n"),
        (
            "xs.map((x) => {\nreturn x;\n})\n",
            "xs.map((x) => {\n  return x;\n})\n",
        ),
        (
            "db.a.aggregate([\n{ $match: {\nx: 1\n} },\n{ $group: {\n_id: 1,\nn: { $sum: 1 }\n} }\n])\n",
            "db.a.aggregate([\n  { $match: {\n    x: 1\n  } },\n  { $group: {\n    _id: 1,\n    n: { $sum: 1 }\n  } }\n])\n",
        ),
        // Already-formatted text stays put, wherever the first bracket opened.
        (
            "if (a) {\n  f(function () {\n    g();\n  }, 1);\n} else {\n  h();\n}\n",
            "if (a) {\n  f(function () {\n    g();\n  }, 1);\n} else {\n  h();\n}\n",
        ),
    ];
    for (src, want) in cases {
        assert_eq!(fmt(src), want, "source: {src:?}");
    }
}

#[test]
fn regex_literals_are_skipped_so_their_quotes_and_brackets_do_not_count() {
    let cases = [
        ("a = /[(]/.test(s)\n  b\n", "a = /[(]/.test(s)\nb\n"),
        ("a = /[\"']/\n  b({\nc\n})\n", "a = /[\"']/\nb({\n  c\n})\n"),
        (
            "a = /\\/\\//; f({\n  b\n})\n",
            "a = /\\/\\//; f({\n  b\n})\n",
        ),
        ("a = /\\/*x/; f({\nb\n})\n", "a = /\\/*x/; f({\n  b\n})\n"),
        (
            "x = /[{]/.test(s)\ny\nz({\nw\n})\n",
            "x = /[{]/.test(s)\ny\nz({\n  w\n})\n",
        ),
        // Division is not a regex, even with a quote or bracket between the slashes.
        (
            "a = b / c / d;\n  f({\nx\n})\n",
            "a = b / c / d;\nf({\n  x\n})\n",
        ),
        (
            "a = b / '(' / d;\n  f({\nx\n})\n",
            "a = b / '(' / d;\nf({\n  x\n})\n",
        ),
        // No closing slash on the line: a division.
        ("a = (b / c\n  d)\n", "a = (b / c\n  d)\n"),
    ];
    for (src, want) in cases {
        assert_eq!(fmt(src), want, "source: {src:?}");
    }
}

#[test]
fn a_cursor_on_the_first_character_stays_on_it_when_the_indent_grows() {
    let src = "f({\na\n})\n";
    let at = src.find('a').unwrap();
    let got = fmt_sel(src, at..at + 1);
    assert_eq!(got.text, "f({\n  a\n})\n");
    let sel = got.selection.unwrap();
    assert_eq!(&got.text[sel.clone()], "a");
    let got = fmt_sel(src, at..at);
    assert_eq!(got.selection, Some(6..6));
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
    "x = `a   \n   b  \n\t${ 1 }  \n   c`  \n  y  \n",
    "/* a  \n   b  \n\t*/ z  \n  w\n",
    "s = 'a\\\n   b  \\\r\n  c'\n  t\n",
    "x = /[(]/.test(s)\n  y = 1\n/\"/ + `\n  q  \n`\n",
];

const TOKENS: &[&str] = &[
    "{", "}", "(", ")", "[", "]", "'", "\"", "`", "//", "/*", "*/", "\n", "\n", "\r\n", " ", "  ",
    "\t", "a", "b1", "${", "}", "\\", "é", "😀", ",", ";", "/", "=", "(/", "/[(]/", "/\\//",
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

/// An independent oracle (a plain char loop, nothing shared with the production scanner): for
/// each `\n`-separated line, whether it begins inside a string continuation, a template literal
/// or a block comment. Such a line is literal text and must come out byte-identical.
fn literal_line_starts(text: &str) -> Vec<bool> {
    #[derive(PartialEq)]
    enum S {
        Code,
        Sq,
        Dq,
        Tpl,
        Line,
        Block,
    }
    let cs: Vec<char> = text.chars().collect();
    let at = |i: usize| cs.get(i).copied();
    let eol = |i: usize| at(i) == Some('\n') || (at(i) == Some('\r') && at(i + 1) == Some('\n'));
    let mut starts = vec![false];
    let mut st = S::Code;
    // Per open `${`: how many plain `{` are open inside it.
    let mut subst: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\n' {
            if matches!(st, S::Line | S::Sq | S::Dq) {
                st = S::Code;
            }
            starts.push(st != S::Code);
            i += 1;
            continue;
        }
        match st {
            S::Code => match c {
                '\'' => st = S::Sq,
                '"' => st = S::Dq,
                '`' => st = S::Tpl,
                '/' if at(i + 1) == Some('/') => {
                    st = S::Line;
                    i += 1;
                }
                '/' if at(i + 1) == Some('*') => {
                    st = S::Block;
                    i += 1;
                }
                '/' => {
                    let mut k = i;
                    let prev = loop {
                        if k == 0 || cs[k - 1] == '\n' {
                            break None;
                        }
                        k -= 1;
                        if cs[k] != ' ' && cs[k] != '\t' {
                            break Some(cs[k]);
                        }
                    };
                    if prev.is_none_or(|p| "(,=:[{!&|?;+".contains(p)) {
                        let (mut j, mut class) = (i + 1, false);
                        while j < cs.len() && !eol(j) {
                            match cs[j] {
                                '\\' if eol(j + 1) => break,
                                '\\' => j += 1,
                                '[' => class = true,
                                ']' => class = false,
                                '/' if !class => {
                                    i = j;
                                    break;
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                    }
                }
                '{' => {
                    if let Some(n) = subst.last_mut() {
                        *n += 1;
                    }
                }
                '}' => match subst.last_mut() {
                    Some(0) => {
                        subst.pop();
                        st = S::Tpl;
                    }
                    Some(n) => *n -= 1,
                    None => {}
                },
                _ => {}
            },
            S::Sq | S::Dq | S::Tpl => {
                let quote = match st {
                    S::Sq => '\'',
                    S::Dq => '"',
                    _ => '`',
                };
                if c == '\\' && !eol(i + 1) {
                    i += 1;
                } else if c == '\\' {
                    // A backslash at the end of a line continues the literal onto the next one.
                    i += if at(i + 1) == Some('\r') { 2 } else { 1 };
                    starts.push(true);
                } else if c == quote {
                    st = S::Code;
                } else if st == S::Tpl && c == '$' && at(i + 1) == Some('{') {
                    subst.push(0);
                    st = S::Code;
                    i += 1;
                }
            }
            S::Line => {}
            S::Block => {
                if c == '*' && at(i + 1) == Some('/') {
                    st = S::Code;
                    i += 1;
                }
            }
        }
        i += 1;
    }
    starts
}

/// Every line that begins inside a literal is byte-identical in the output.
fn assert_literal_lines_kept(label: &str, src: &str, out: &str) {
    let (src_lines, out_lines): (Vec<_>, Vec<_>) =
        (src.split('\n').collect(), out.split('\n').collect());
    assert_eq!(
        src_lines.len(),
        out_lines.len(),
        "line count: {label}: {src:?}"
    );
    let starts = literal_line_starts(src);
    assert_eq!(
        starts.len(),
        src_lines.len(),
        "oracle lines: {label}: {src:?}"
    );
    for (n, literal) in starts.iter().enumerate() {
        if *literal {
            assert_eq!(
                src_lines[n], out_lines[n],
                "literal line {n} changed: {label}: {src:?}"
            );
        }
    }
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
    assert_literal_lines_kept(label, src, &once);
    // Formatting arbitrary ranges (deliberately not aligned to chars or lines) must not panic.
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
        assert_literal_lines_kept(label, src, &got.text);
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
