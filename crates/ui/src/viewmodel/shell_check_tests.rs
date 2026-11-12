use super::*;
use crate::views::shell_highlight::{Kind, tokens};

fn kind(text: &str) -> Option<ProblemKind> {
    check(text).map(|p| p.kind)
}

#[test]
fn balanced_script_has_no_problem() {
    assert_eq!(check("db.a.find({})"), None);
    assert_eq!(check(""), None);
    assert_eq!(check("f([1, (2)], {a: [3]})\n"), None);
}

#[test]
fn unclosed_bracket_points_at_the_opener() {
    let p = check("db.a.find({").expect("problem");
    assert_eq!(p.kind, ProblemKind::UnclosedBracket('{'));
    assert_eq!(p.range, 10..11);
    assert_eq!((p.line, p.column), (1, 11));
}

#[test]
fn the_innermost_unclosed_bracket_is_reported() {
    assert_eq!(kind("f(\n[ {"), Some(ProblemKind::UnclosedBracket('{')));
}

#[test]
fn unexpected_and_mismatched_closing_brackets() {
    assert_eq!(kind("x )"), Some(ProblemKind::UnexpectedClosing(')')));
    assert_eq!(
        kind("( ]"),
        Some(ProblemKind::MismatchedClosing {
            opener: '(',
            found: ']'
        })
    );
    let p = check("a\n  ( ]").expect("problem");
    assert_eq!((p.line, p.column, p.range), (2, 5, 6..7));
}

#[test]
fn unterminated_literals() {
    assert_eq!(kind("'abc"), Some(ProblemKind::UnterminatedString));
    assert_eq!(
        kind("x = \"abc\ny = 1"),
        Some(ProblemKind::UnterminatedString)
    );
    assert_eq!(kind("`a${b}"), Some(ProblemKind::UnterminatedTemplate));
    assert_eq!(kind("/* x"), Some(ProblemKind::UnterminatedComment));
    assert_eq!(kind("'a\\\nb'"), None, "a backslash continues a string");
    assert_eq!(kind("`a\nb`"), None, "templates span lines");
}

#[test]
fn brackets_in_literals_and_comments_are_not_counted() {
    assert_eq!(check("'(' + \"[\" + `{`"), None);
    assert_eq!(check("x // ( [ {\ny"), None);
    assert_eq!(check("/* ( */ x"), None);
    assert_eq!(check("`a(${b})`"), None);
    assert_eq!(check("x = /[(]/"), None);
    assert_eq!(check("f(/\\)/)"), None);
    assert_eq!(check("x = /[\"(]/"), None, "quote and bracket in a regex");
}

#[test]
fn an_unterminated_string_in_a_placeholder_is_reported() {
    assert_eq!(kind("`a${ 'x\n}`"), Some(ProblemKind::UnterminatedString));
}

#[test]
fn an_earlier_unclosed_opener_wins_over_a_later_swallowing_string() {
    // The `'` swallows the `)` that would close the `(`, but the `(` comes first.
    assert_eq!(kind("f(\n'a )"), Some(ProblemKind::UnclosedBracket('(')));
}

#[test]
fn a_placeholders_brackets_are_counted() {
    assert_eq!(
        kind("`a${ f( }`"),
        Some(ProblemKind::MismatchedClosing {
            opener: '(',
            found: '}'
        })
    );
    assert_eq!(kind("`a${b"), Some(ProblemKind::UnclosedBracket('{')));
    assert_eq!(kind("f(`${ x }`"), Some(ProblemKind::UnclosedBracket('(')));
    assert_eq!(kind("`${ {a:1}.a }`"), None);
    assert_eq!(
        kind("`${ ]`"),
        Some(ProblemKind::MismatchedClosing {
            opener: '{',
            found: ']'
        })
    );
}

#[test]
fn the_first_problem_in_text_order_wins() {
    // The unterminated string comes before the stray bracket.
    assert_eq!(kind("'a\n)"), Some(ProblemKind::UnterminatedString));
    assert_eq!(kind(") 'a"), Some(ProblemKind::UnexpectedClosing(')')));
    assert_eq!(kind("( `a"), Some(ProblemKind::UnclosedBracket('(')));
}

#[test]
fn crlf_input_reports_the_right_line() {
    let p = check("a\r\nb\r\n  (").expect("problem");
    assert_eq!((p.line, p.column), (3, 3));
}

#[test]
fn column_counts_chars_not_bytes() {
    let p = check("'é' + (").expect("problem");
    assert_eq!(p.column, 7);
    assert_eq!(p.range, 7..8, "the range is in bytes");
}

const CORPUS: &[&str] = &[
    "db.users.find({ name: 'a(b' }) // note )\n",
    "const re = /a\\/*b/;\nlet s = \"x\\\"y\";",
    "/* multi\nline ( */ db.a.aggregate([{ $match: { x: `t${1 + (2)}u` } }])",
    "x = 'a\\\nb'; y = `line1\nline2 ${ f({a: 1}) } end`",
    "a / b / c; if (x) { /[}]/.test(y) }",
    "'unterminated\nnext(\"ok\")",
    "`open ${ {a:1} ",
    "show dbs; use shop; it",
    "x = /[\"(]/; f(`a${`b${c}d`}e`)",
    "} /x/ y\n/* a\n*/ /x/ ' z",
    "'a\\\r\nb' + 1",
];

#[test]
fn check_agrees_with_the_highlighter_on_where_strings_and_comments_are() {
    for src in CORPUS {
        let scan = crate::viewmodel::shell_scan::scan_code(src);
        let mut in_literal = vec![false; src.len()];
        for (kind, range) in tokens(src) {
            if !matches!(kind, Kind::String | Kind::Comment) {
                assert!(
                    range.clone().all(|i| scan.in_code[i]),
                    "{kind:?} at {range:?} of {src:?} is code to the highlighter but not to the check"
                );
                continue;
            }
            for i in range {
                in_literal[i] = true;
                // Only a placeholder's `{` and `}` belong to the template for the highlighter
                // yet are brackets for the check; everything else must agree.
                let placeholder_brace = scan.in_code[i] && matches!(src.as_bytes()[i], b'{' | b'}');
                assert!(
                    !scan.in_code[i] || placeholder_brace,
                    "byte {i} of {src:?} is {kind:?} to the highlighter but code to the check"
                );
            }
        }
        // And back: what the check calls non-code lies inside a string, comment or regex token.
        for (i, &code) in scan.in_code.iter().enumerate() {
            assert!(
                code || in_literal[i],
                "byte {i} of {src:?} is not code to the check but not in a literal to the highlighter"
            );
        }
    }
}
