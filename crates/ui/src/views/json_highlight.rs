//! Syntax colors for one line of the JSON view. The lines come from the pretty printer, so a
//! token never spans lines: a string ends on its line, and a string followed by a colon is a key.
use crate::theme::JsonColors;
use eframe::egui::{Color32, FontId, text::LayoutJob, text::TextFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Key,
    String,
    Number,
    Boolean,
    Null,
    Punctuation,
}

const PUNCTUATION: &[char] = &['{', '}', '[', ']', ',', ':'];

/// `line` as colored sections: `plain` for whitespace and unknown words, `colors` for tokens.
pub fn highlight(line: &str, font: &FontId, plain: Color32, colors: &JsonColors) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (kind, text) in tokens(line) {
        let color = match kind {
            Kind::Plain => plain,
            Kind::Key => colors.key,
            Kind::String => colors.string,
            Kind::Number => colors.number,
            Kind::Boolean => colors.boolean,
            Kind::Null => colors.null,
            Kind::Punctuation => colors.punctuation,
        };
        job.append(text, 0.0, TextFormat::simple(font.clone(), color));
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

fn tokens(line: &str) -> Vec<(Kind, &str)> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(first) = rest.chars().next() {
        let (kind, len) = if first == '"' {
            let len = string_len(rest);
            let key = rest[len..].trim_start().starts_with(':');
            (if key { Kind::Key } else { Kind::String }, len)
        } else if PUNCTUATION.contains(&first) {
            (Kind::Punctuation, first.len_utf8())
        } else if first.is_whitespace() {
            (Kind::Plain, run_len(rest, char::is_whitespace))
        } else {
            let len = run_len(rest, |c| {
                !c.is_whitespace() && c != '"' && !PUNCTUATION.contains(&c)
            });
            (word_kind(&rest[..len]), len)
        };
        out.push((kind, &rest[..len]));
        rest = &rest[len..];
    }
    out
}

/// Byte length of the string at the start of `s` (which starts with `"`), closing quote
/// included; an unterminated string runs to the end of the line.
fn string_len(s: &str) -> usize {
    let mut escaped = false;
    for (index, c) in s.char_indices().skip(1) {
        match (escaped, c) {
            (true, _) => escaped = false,
            (false, '\\') => escaped = true,
            (false, '"') => return index + 1,
            _ => {}
        }
    }
    s.len()
}

fn run_len(s: &str, keep: impl Fn(char) -> bool) -> usize {
    s.find(|c| !keep(c)).unwrap_or(s.len())
}

fn word_kind(word: &str) -> Kind {
    match word {
        "true" | "false" => Kind::Boolean,
        "null" => Kind::Null,
        _ if word.starts_with(|c: char| c.is_ascii_digit() || c == '-') => Kind::Number,
        _ => Kind::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(Kind, &str)> {
        tokens(line)
            .into_iter()
            .filter(|(kind, _)| *kind != Kind::Plain)
            .collect()
    }

    #[test]
    fn a_key_and_its_value_are_told_apart() {
        assert_eq!(
            kinds(r#"  "name": "Ada","#),
            [
                (Kind::Key, r#""name""#),
                (Kind::Punctuation, ":"),
                (Kind::String, r#""Ada""#),
                (Kind::Punctuation, ","),
            ]
        );
    }

    #[test]
    fn numbers_booleans_and_null_get_their_own_kind() {
        assert_eq!(
            kinds("1.5e3,"),
            [(Kind::Number, "1.5e3"), (Kind::Punctuation, ",")]
        );
        assert_eq!(kinds("-7"), [(Kind::Number, "-7")]);
        assert_eq!(kinds("true"), [(Kind::Boolean, "true")]);
        assert_eq!(kinds("false"), [(Kind::Boolean, "false")]);
        assert_eq!(kinds("null"), [(Kind::Null, "null")]);
    }

    #[test]
    fn escaped_quotes_and_colons_stay_inside_a_string() {
        assert_eq!(
            kinds(r#""say \"hi\": ok""#),
            [(Kind::String, r#""say \"hi\": ok""#)]
        );
    }

    #[test]
    fn an_unterminated_string_runs_to_the_end_and_text_is_never_lost() {
        let line = r#"  "open"#;
        assert_eq!(kinds(line), [(Kind::String, r#""open"#)]);
        for line in [
            line,
            "\u{e9}\"\u{e9}\": \u{e9}",
            "",
            "   ",
            r#"{"a":[1,2]}"#,
        ] {
            let text: String = tokens(line).into_iter().map(|(_, t)| t).collect();
            assert_eq!(text, line);
        }
    }

    #[test]
    fn the_job_carries_the_colors_and_the_whole_line() {
        let colors = crate::theme::json_colors(crate::theme::scheme(crate::theme::Mode::Light));
        let job = highlight(
            r#""a": null"#,
            &FontId::monospace(12.0),
            Color32::BLACK,
            &colors,
        );
        assert_eq!(job.text, r#""a": null"#);
        let used: Vec<Color32> = job.sections.iter().map(|s| s.format.color).collect();
        assert!(used.contains(&colors.key) && used.contains(&colors.null));
    }
}
