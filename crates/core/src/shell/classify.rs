//! Turns `mongosh` stdout (already decoded and scrubbed) into what the Shell tab shows.
//!
//! `mongosh --nodb --quiet` reading piped stdin behaves like its REPL (see
//! `docs/superpowers/notes/shell-stdin-spike.md`): it prints a prompt before each line it reads,
//! and errors on stdout as `Uncaught \r<Error>` or `Uncaught:\n\r<SyntaxError>`. The preamble sets
//! a fixed prompt ([`PROMPT`]) before it connects, so the classifier
//! - hides everything up to the preamble's `__CB_READY__` / `__CB_FAILED__ <message>` line
//!   (printed after the default prompt, `> `), plus the blank line `print` leaves after it; when
//!   the output ends without either, it reports the last lines it hid ([`Piece::NoSentinel`]);
//! - strips every [`PROMPT`], wherever it appears, and every `\r`;
//! - strips the continuation prompts (`| `, one per continuation line) that directly follow a
//!   [`PROMPT`], so an error after them still starts its line;
//! - routes a block that starts (at a line start) with the error marker to stderr until the next
//!   prompt. The marker includes the `\r`, so printed text such as `Uncaught x` stays output.
//!
//! Limitation: a script that redefines `prompt` gets its own prompt shown as output.
use super::Stream;
use super::invocation::PROMPT;
use std::collections::VecDeque;

const READY: &str = "__CB_READY__";
const FAILED: &str = "__CB_FAILED__";
/// What `mongosh` prints (no newline) before it reads each continuation line of a statement
/// that is not complete yet, so a multi-line statement's output starts with one per line.
const CONTINUATION: &str = "| ";
/// How `mongosh` starts an error block (runtime error, syntax error).
const ERROR_MARKERS: [&str; 2] = ["Uncaught \r", "Uncaught:\n\r"];
/// Lines of pre-sentinel output kept to explain a run that never connected.
const UNREADY_LINES: usize = 20;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Piece {
    Out(Stream, String),
    /// The preamble connected; the script may be written.
    Ready,
    /// The preamble failed with this message; nothing after it is shown.
    Failed(String),
    /// Output ended before either sentinel: the last lines seen (each ending in `\n`).
    NoSentinel(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Handshake,
    Running,
    Done,
}

#[derive(Debug, Default)]
pub(crate) struct Classifier {
    phase: Phase,
    /// Text not classified yet: an unfinished handshake line, or text that could still become
    /// a prompt or an error marker.
    held: String,
    /// The last hidden pre-sentinel lines.
    unready: VecDeque<String>,
    /// False while the next text starts a line.
    mid_line: bool,
    /// Only prompts and continuation prompts since the last [`PROMPT`].
    after_prompt: bool,
    in_error: bool,
    /// Drop one `\n` right after the READY line (the empty result `print` leaves).
    skip_blank: bool,
    saw_uncaught: bool,
}

impl Classifier {
    pub(crate) fn push(&mut self, text: &str) -> Vec<Piece> {
        self.feed(text, false)
    }

    /// Releases what is held; call once when stdout ends.
    pub(crate) fn finish(&mut self) -> Vec<Piece> {
        self.feed("", true)
    }

    /// Whether any error block was seen.
    pub(crate) fn saw_uncaught(&self) -> bool {
        self.saw_uncaught
    }

    fn feed(&mut self, text: &str, at_end: bool) -> Vec<Piece> {
        let mut out = Vec::new();
        match self.phase {
            Phase::Handshake => self.handshake(text, at_end, &mut out),
            Phase::Running => self.run(text, at_end, &mut out),
            Phase::Done => {}
        }
        out
    }

    fn handshake(&mut self, text: &str, at_end: bool, out: &mut Vec<Piece>) {
        self.held.push_str(text);
        if at_end && !self.held.is_empty() && !self.held.ends_with('\n') {
            self.held.push('\n');
        }
        while let Some(nl) = self.held.find('\n') {
            let line = self.held[..nl].replace('\r', "");
            let rest = self.held[nl + 1..].to_owned();
            if after_prompts(&line, READY) == Some("") {
                self.phase = Phase::Running;
                self.held.clear();
                self.unready.clear();
                self.skip_blank = true;
                out.push(Piece::Ready);
                self.run(&rest, at_end, out);
                return;
            }
            if let Some(message) = after_prompts(&line, FAILED) {
                if message.is_empty() || message.starts_with(' ') {
                    self.phase = Phase::Done;
                    self.held.clear();
                    out.push(Piece::Failed(message.trim_start().to_owned()));
                    return;
                }
            }
            self.remember_unready(&line);
            self.held = rest;
        }
        if at_end {
            self.phase = Phase::Done;
            let lines: String = self.unready.drain(..).map(|l| l + "\n").collect();
            out.push(Piece::NoSentinel(lines));
        }
    }

    fn remember_unready(&mut self, line: &str) {
        let mut line = line;
        while let Some(rest) = line.strip_prefix("> ") {
            line = rest;
        }
        if line.trim().is_empty() {
            return;
        }
        if self.unready.len() == UNREADY_LINES {
            self.unready.pop_front();
        }
        self.unready.push_back(line.to_owned());
    }

    fn run(&mut self, text: &str, at_end: bool, out: &mut Vec<Piece>) {
        let mut input = std::mem::take(&mut self.held);
        input.push_str(text);
        let mut rest = input.as_str();
        if self.skip_blank && !rest.is_empty() {
            self.skip_blank = false;
            rest = rest.strip_prefix('\n').unwrap_or(rest);
        }
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix(PROMPT) {
                self.in_error = false;
                self.mid_line = false;
                self.after_prompt = true;
                rest = after;
                continue;
            }
            if !at_end && PROMPT.starts_with(rest) {
                self.held = rest.to_owned();
                return;
            }
            if self.after_prompt {
                if let Some(after) = rest.strip_prefix(CONTINUATION) {
                    rest = after;
                    continue;
                }
                if !at_end && CONTINUATION.starts_with(rest) {
                    self.held = rest.to_owned();
                    return;
                }
                self.after_prompt = false;
            }
            if !self.mid_line {
                if ERROR_MARKERS.iter().any(|m| rest.starts_with(m)) {
                    self.in_error = true;
                    self.saw_uncaught = true;
                } else if !at_end && ERROR_MARKERS.iter().any(|m| m.starts_with(rest)) {
                    self.held = rest.to_owned();
                    return;
                }
                self.mid_line = true;
            }
            rest = self.emit_segment(rest, out);
        }
    }

    /// Emits `rest` up to and including the next newline, or up to the next possible prompt;
    /// returns what is left.
    fn emit_segment<'a>(&mut self, rest: &'a str, out: &mut Vec<Piece>) -> &'a str {
        let stream = if self.in_error {
            Stream::Stderr
        } else {
            Stream::Stdout
        };
        // A leading U+0001 here is not a prompt (checked by the caller): it is output.
        let skip = usize::from(rest.starts_with('\u{1}'));
        match rest[skip..].find(['\n', '\u{1}']).map(|i| i + skip) {
            Some(i) if rest[i..].starts_with('\n') => {
                emit(out, stream, &rest[..=i]);
                self.mid_line = false;
                &rest[i + 1..]
            }
            Some(i) => {
                emit(out, stream, &rest[..i]);
                &rest[i..]
            }
            None => {
                emit(out, stream, rest);
                ""
            }
        }
    }
}

/// The text after `token` when everything before it in `line` is default prompts
/// (`> `, `admin> `, `rs0 [primary] admin> `, `[mongos] admin> `, `Atlas ... [primary] admin> `).
fn after_prompts<'a>(line: &'a str, token: &str) -> Option<&'a str> {
    let at = line.find(token)?;
    let prefix = &line[..at];
    let is_prompts = prefix.is_empty()
        || (prefix.ends_with("> ")
            && prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-[]: >".contains(&b)));
    is_prompts.then(|| &line[at + token.len()..])
}

/// Appends text without `\r`, merging with the previous piece when it is on the same stream.
fn emit(out: &mut Vec<Piece>, stream: Stream, text: &str) {
    let text = text.replace('\r', "");
    if text.is_empty() {
        return;
    }
    if let Some(Piece::Out(last, existing)) = out.last_mut() {
        if *last == stream {
            existing.push_str(&text);
            return;
        }
    }
    out.push(Piece::Out(stream, text));
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
