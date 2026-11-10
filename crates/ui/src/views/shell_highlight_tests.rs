use super::*;
use crate::theme::{Mode, js_colors, scheme};
use eframe::egui::{Context, FontId};

fn kinds(text: &str) -> Vec<(Kind, &str)> {
    tokens(text)
        .into_iter()
        .filter(|(kind, _)| *kind != Kind::Plain)
        .map(|(kind, range)| (kind, &text[range]))
        .collect()
}

fn colors() -> JsColors {
    js_colors(scheme(Mode::Light))
}

#[test]
fn a_find_call_marks_db_as_a_shell_word_and_leaves_the_rest_plain() {
    assert_eq!(
        kinds("db.users.find({})"),
        [(Kind::ShellWord, "db")],
        "users and find are property names, the braces are punctuation"
    );
}

#[test]
fn shell_words_are_only_marked_where_they_are_not_property_names() {
    assert_eq!(
        kinds("use shop; show dbs; sh.status(); rs.conf(); it; x.db; a.use"),
        [
            (Kind::ShellWord, "use"),
            (Kind::ShellWord, "show"),
            (Kind::ShellWord, "sh"),
            (Kind::ShellWord, "rs"),
            (Kind::ShellWord, "it"),
        ]
    );
}

#[test]
fn keywords_are_marked_but_names_that_contain_them_are_not() {
    assert_eq!(
        kinds("const forEach = function () { return null }"),
        [
            (Kind::Keyword, "const"),
            (Kind::Keyword, "function"),
            (Kind::Keyword, "return"),
            (Kind::Keyword, "null"),
        ]
    );
}

#[test]
fn brackets_and_comment_markers_stay_inside_strings() {
    assert_eq!(
        kinds(r#"print("a { [ // b", 'c } /* d', "e \" f")"#),
        [
            (Kind::String, r#""a { [ // b""#),
            (Kind::String, "'c } /* d'"),
            (Kind::String, r#""e \" f""#),
        ]
    );
}

#[test]
fn an_unterminated_string_ends_at_the_end_of_its_line() {
    assert_eq!(
        kinds("x = \"open\ny = 1"),
        [(Kind::String, "\"open"), (Kind::Number, "1")]
    );
}

#[test]
fn template_literals_span_lines_and_their_placeholders_are_code() {
    assert_eq!(
        kinds("`a\n${db.n + 1} {b}`"),
        [
            (Kind::String, "`a\n${"),
            (Kind::ShellWord, "db"),
            (Kind::Number, "1"),
            (Kind::String, "} {b}`"),
        ]
    );
}

#[test]
fn braces_inside_a_placeholder_do_not_close_it() {
    assert_eq!(
        kinds("`${ {a: 1}.a }x`"),
        [
            (Kind::String, "`${"),
            (Kind::Number, "1"),
            (Kind::String, "}x`"),
        ]
    );
}

#[test]
fn line_and_block_comments_are_marked() {
    assert_eq!(
        kinds("a // note \"x\"\n/* one\ntwo */ 2"),
        [
            (Kind::Comment, "// note \"x\""),
            (Kind::Comment, "/* one\ntwo */"),
            (Kind::Number, "2"),
        ]
    );
    assert_eq!(kinds("/* open"), [(Kind::Comment, "/* open")]);
}

#[test]
fn numbers_of_every_shape_are_marked() {
    assert_eq!(
        kinds("[1, 2.5, 0x1F, 1e3, 1_000]"),
        [
            (Kind::Number, "1"),
            (Kind::Number, "2.5"),
            (Kind::Number, "0x1F"),
            (Kind::Number, "1e3"),
            (Kind::Number, "1_000"),
        ]
    );
}

#[test]
fn deeply_nested_templates_do_not_overflow_the_stack() {
    let text = "`${".repeat(100_000);
    let job = highlight_js(&text, FontId::monospace(12.0), &colors());
    assert_eq!(job.text, text);
}

#[test]
fn the_job_holds_the_text_exactly_whatever_it_is() {
    for text in [
        "",
        " ",
        "\n\n",
        "db.users.find({ name: \"Zoë 日本\" }) // ünïcode ✓",
        "'\u{e9}",
        "`${\u{1F600}`",
        "/*",
        "\"\\",
        "a\r\nb",
    ] {
        let job = highlight_js(text, FontId::monospace(12.0), &colors());
        assert_eq!(job.text, text);
        assert!(
            !job.sections.is_empty(),
            "{text:?} needs a section for its font"
        );
        let covered: usize = job.sections.iter().map(|s| s.byte_range.len()).sum();
        assert_eq!(covered, text.len(), "{text:?}");
    }
}

#[test]
fn each_kind_gets_its_color() {
    let c = colors();
    let job = highlight_js("db // x", FontId::monospace(12.0), &c);
    let first = job.sections.first().expect("a section");
    let last = job.sections.last().expect("a section");
    assert_eq!(first.format.color, c.shell_word);
    assert_eq!(last.format.color, c.comment);
}

#[test]
fn the_cache_returns_the_same_job_for_the_same_key_and_a_new_one_otherwise() {
    let ctx = Context::default();
    let slot = eframe::egui::Id::new("test_highlight_slot");
    let font = FontId::monospace(12.0);
    let c = colors();
    let first = cached_highlight(&ctx, slot, "db.a.find()", &font, &c, 300.0);
    let again = cached_highlight(&ctx, slot, "db.a.find()", &font, &c, 300.0);
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(first.wrap.max_width, 300.0);
    let wider = cached_highlight(&ctx, slot, "db.a.find()", &font, &c, 400.0);
    assert!(!Arc::ptr_eq(&first, &wider));
    let edited = cached_highlight(&ctx, slot, "db.b.find()", &font, &c, 400.0);
    assert!(!Arc::ptr_eq(&wider, &edited));
    let dark = js_colors(scheme(Mode::Dark));
    let themed = cached_highlight(&ctx, slot, "db.b.find()", &font, &dark, 400.0);
    assert!(!Arc::ptr_eq(&edited, &themed));
}
