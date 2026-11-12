//! Syntax colors for the Shell tab's editor: a small, forgiving JavaScript lexer that never
//! fails and never loses text. It marks keywords, strings, template literals (their `${}`
//! placeholders are code again), comments, numbers and the mongosh words (`db`, `sh`, `rs`,
//! `use`, `show`, `it`); everything else is plain. A regular expression literal is told from a
//! division the way the formatter tells it (`shell_format::starts_regex`) and colored as a string.
use crate::theme::JsColors;
use crate::viewmodel::shell_format::{regex_end, starts_regex};
use eframe::egui::{self, FontId, text::LayoutJob, text::TextFormat};
use std::ops::Range;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Plain,
    Keyword,
    String,
    Number,
    Comment,
    ShellWord,
}

const KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

const SHELL_WORDS: &[&str] = &["db", "sh", "rs", "use", "show", "it"];

/// Template literals nested deeper than this inside `${}` are colored as plain string text, so
/// a pathological script cannot exhaust the stack.
const MAX_TEMPLATE_DEPTH: usize = 32;

/// `text` as colored sections in `font`. The job's text is `text` exactly; it always has at
/// least one section, so even an empty editor knows its row height.
pub fn highlight_js(text: &str, font: FontId, colors: &JsColors) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (kind, range) in tokens(text) {
        let format = TextFormat::simple(font.clone(), color(kind, colors));
        job.append(&text[range], 0.0, format);
    }
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat::simple(font, colors.plain));
    }
    job
}

/// [`highlight_js`] laid out for `wrap_width`, kept in egui memory: the editor's layouter runs
/// every frame, and only a change of text, font, colors or width lexes again. `slot` is the
/// memory slot (one per editor), which holds the last job only.
pub fn cached_highlight(
    ctx: &egui::Context,
    slot: egui::Id,
    text: &str,
    font: &FontId,
    colors: &JsColors,
    wrap_width: f32,
) -> Arc<LayoutJob> {
    let key = egui::util::hash((text, font, colors, wrap_width.to_bits()));
    let id = slot;
    let hit = ctx
        .data(|d| d.get_temp::<CachedJob>(id))
        .filter(|cached| cached.key == key);
    if let Some(cached) = hit {
        return cached.job;
    }
    let mut job = highlight_js(text, font.clone(), colors);
    job.wrap.max_width = wrap_width;
    let job = Arc::new(job);
    let cached = CachedJob {
        key,
        job: Arc::clone(&job),
    };
    ctx.data_mut(|d| d.insert_temp(id, cached));
    job
}

#[derive(Clone)]
struct CachedJob {
    key: u64,
    job: Arc<LayoutJob>,
}

fn color(kind: Kind, colors: &JsColors) -> egui::Color32 {
    match kind {
        Kind::Plain => colors.plain,
        Kind::Keyword => colors.keyword,
        Kind::String => colors.string,
        Kind::Number => colors.number,
        Kind::Comment => colors.comment,
        Kind::ShellWord => colors.shell_word,
    }
}

/// The tokens of `text` in order, covering every byte; neighbours of one kind are merged.
pub(crate) fn tokens(text: &str) -> Vec<(Kind, Range<usize>)> {
    let mut lexer = Lexer {
        text,
        pos: 0,
        out: Vec::new(),
    };
    lexer.code(0);
    lexer.out
}

struct Lexer<'a> {
    text: &'a str,
    pos: usize,
    out: Vec<(Kind, Range<usize>)>,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.text[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn emit(&mut self, kind: Kind, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        match self.out.last_mut() {
            Some((last, prev)) if *last == kind && prev.end == range.start => prev.end = range.end,
            _ => self.out.push((kind, range)),
        }
    }

    /// Code up to the end, or (inside the placeholder of a template at `depth` > 0) up to the
    /// `}` that closes it, which is left for the template.
    fn code(&mut self, depth: usize) {
        let mut braces = 0usize;
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                '/' if self.rest().starts_with("//") => self.line_comment(),
                '/' if self.rest().starts_with("/*") => self.block_comment(),
                '/' if self.regex() => {}
                '"' | '\'' => self.string(c),
                '`' => self.template(depth + 1),
                '}' if depth > 0 && braces == 0 => return,
                _ if c.is_ascii_digit() => self.number(),
                _ if is_word_start(c) => self.word(),
                _ => {
                    braces = match c {
                        '{' => braces + 1,
                        '}' => braces.saturating_sub(1),
                        _ => braces,
                    };
                    self.pos += c.len_utf8();
                    self.emit(Kind::Plain, start..self.pos);
                }
            }
        }
    }

    /// A regex literal at the `/` here, by the formatter's rule (on one line, opened where an
    /// operator or nothing comes before it), colored as a string; false when there is none.
    fn regex(&mut self) -> bool {
        let start = self.pos;
        let line_start = self.text[..start].rfind('\n').map_or(0, |at| at + 1);
        let line_end = self.text[start..]
            .find('\n')
            .map_or(self.text.len(), |at| start + at);
        let line = &self.text.as_bytes()[line_start..line_end];
        let at = start - line_start;
        let Some(end) = starts_regex(line, at)
            .then(|| regex_end(line, at))
            .flatten()
        else {
            return false;
        };
        self.pos = line_start + end + 1;
        self.emit(Kind::String, start..self.pos);
        true
    }

    fn line_comment(&mut self) {
        let start = self.pos;
        self.pos += self.rest().find('\n').unwrap_or(self.rest().len());
        self.emit(Kind::Comment, start..self.pos);
    }

    fn block_comment(&mut self) {
        let start = self.pos;
        self.pos += self.rest()[2..]
            .find("*/")
            .map_or(self.rest().len(), |end| end + 4);
        self.emit(Kind::Comment, start..self.pos);
    }

    /// A quoted string; an unterminated one ends before its line break.
    fn string(&mut self, quote: char) {
        let start = self.pos;
        let mut chars = self.rest().char_indices().skip(1);
        let mut end = self.rest().len();
        while let Some((at, c)) = chars.next() {
            match c {
                // The escaped character, a line break included (`\r\n` is one), belongs to the string.
                '\\' => {
                    let crlf = self.rest().as_bytes().get(at + 1..at + 3) == Some(b"\r\n");
                    chars.next();
                    if crlf {
                        chars.next();
                    }
                }
                '\n' => {
                    end = at;
                    break;
                }
                _ if c == quote => {
                    end = at + c.len_utf8();
                    break;
                }
                _ => {}
            }
        }
        self.pos += end;
        self.emit(Kind::String, start..self.pos);
    }

    /// A template literal; `${...}` placeholders are lexed as code until their closing brace.
    fn template(&mut self, depth: usize) {
        let mut start = self.pos;
        self.pos += 1;
        loop {
            let Some(c) = self.peek() else {
                self.emit(Kind::String, start..self.pos);
                return;
            };
            match c {
                '\\' => {
                    self.pos += 1;
                    self.pos += self.peek().map_or(0, char::len_utf8);
                }
                '`' => {
                    self.pos += 1;
                    self.emit(Kind::String, start..self.pos);
                    return;
                }
                '$' if self.rest().starts_with("${") && depth < MAX_TEMPLATE_DEPTH => {
                    self.pos += 2;
                    self.emit(Kind::String, start..self.pos);
                    self.code(depth);
                    // `code` stops on the closing brace or at the end of the text.
                    start = self.pos;
                    if self.peek().is_none() {
                        return;
                    }
                    self.pos += 1;
                }
                _ => self.pos += c.len_utf8(),
            }
        }
    }

    /// Decimal, hex, exponent and `_`-separated numbers alike: a digit and what follows it.
    fn number(&mut self) {
        let start = self.pos;
        self.pos += run_len(self.rest(), |c| {
            c.is_ascii_alphanumeric() || c == '_' || c == '.'
        });
        self.emit(Kind::Number, start..self.pos);
    }

    fn word(&mut self) {
        let start = self.pos;
        self.pos += run_len(self.rest(), is_word_char);
        let word = &self.text[start..self.pos];
        let before = self.text[..start].trim_end();
        let property = before.ends_with('.') && !before.ends_with("...");
        let kind = match word {
            _ if property => Kind::Plain,
            _ if KEYWORDS.contains(&word) => Kind::Keyword,
            _ if SHELL_WORDS.contains(&word) => Kind::ShellWord,
            _ => Kind::Plain,
        };
        self.emit(kind, start..self.pos);
    }
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn run_len(s: &str, keep: impl Fn(char) -> bool) -> usize {
    s.find(|c| !keep(c)).unwrap_or(s.len())
}

#[cfg(test)]
#[path = "shell_highlight_tests.rs"]
mod tests;
