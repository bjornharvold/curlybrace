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

#[test]
fn a_regex_with_a_quote_and_a_bracket_is_not_code() {
    assert_eq!(code_of("x = /[\"(]/"), "x = ......");
    assert!(scan_code("x = /[\"(]/").unterminated.is_empty());
}

#[test]
fn a_template_nested_in_a_placeholder_returns_to_the_outer_template() {
    // outer text, `${`, an inner template with its own placeholder, then more outer text.
    assert_eq!(code_of("`a${`b${c}d`}e`"), "...{...{c}..}..");
    assert!(scan_code("`a${`b${c}d`}e`").unterminated.is_empty());
}

#[test]
fn an_unterminated_string_in_a_placeholder_is_reported_at_its_quote() {
    let scan = scan_code("`a${ 'x\n}`");
    assert_eq!(scan.unterminated, [(Literal::String, 5)]);
}

#[test]
fn an_escaped_crlf_continues_a_string() {
    let text = "'a\\\r\nb' + c";
    assert!(scan_code(text).unterminated.is_empty());
    assert_eq!(code_of(text), "....... + c");
}

#[test]
fn a_regex_after_a_closing_brace_or_paren_is_a_division() {
    // `starts_regex` reads a `/` after `}` or `)` as division; so does the scan.
    assert_eq!(code_of("} /x/ y"), "} /x/ y");
    assert_eq!(code_of(") /x/ y"), ") /x/ y");
}

#[test]
fn a_slash_on_a_line_that_began_in_a_block_comment_is_judged_on_that_line() {
    // The second line starts inside the comment; the `/` after `*/` follows a `/`, so it is a
    // division, exactly as `format_script` reads that line.
    assert_eq!(code_of("/* a\n*/ /x/ y"), "....... /x/ y");
    assert_eq!(code_of("/* a\n*/ x = /[(]/"), "....... x = .....");
}

#[test]
fn one_long_line_of_regex_literals_is_scanned_in_linear_time() {
    // 256 KiB on one line (eight times the 32 KiB that is already slow to notice), with tens of
    // thousands of `/x/` that each could start a regex: a search to the line's end per slash
    // takes many seconds here, one per line takes milliseconds.
    let unit = "a=/x/;";
    let text = unit.repeat(256 * 1024 / unit.len());
    let started = std::time::Instant::now();
    let scan = scan_code(&text);
    let took = started.elapsed();
    assert!(scan.unterminated.is_empty());
    // Every regex is masked: only `a=` and `;` stay code.
    assert_eq!(code_of(&text[..12]), "a=...;a=...;");
    assert!(took.as_secs_f64() < 2.0, "took {took:?}");
}
