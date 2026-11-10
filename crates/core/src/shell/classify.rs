//! Turns `mongosh` stdout (decoded, not masked yet: see `pipe.rs`) into what the Shell tab shows,
//! and tells the stdin writer when `mongosh` wants the next script line.
//!
//! `mongosh --nodb --quiet` reading piped stdin behaves like its REPL (see
//! `docs/superpowers/notes/shell-stdin-spike.md`): it prints the prompt before each statement it
//! reads, `| ` (no newline) before each continuation line of a statement that is not complete yet,
//! and errors on stdout as `Uncaught \r<Error>` or `Uncaught:\n\r<SyntaxError>`. The preamble sets
//! a fixed prompt ([`PROMPT`]) before it connects, and the script goes in as one block statement
//! (`invocation::script_block`), so the classifier
//! - hides everything up to the preamble's `__CB_READY__` / `__CB_FAILED__ <message>` line
//!   (printed after the default prompt, `> `), plus the blank line `print` leaves after it; when
//!   the output ends without either, it reports the last lines it hid ([`Piece::NoSentinel`]);
//! - strips every [`PROMPT`], wherever it appears, and every `\r`;
//! - strips the continuation prompts that directly follow a [`PROMPT`] (an error after them still
//!   starts its line) and reports each ([`Piece::Continue`]: send the next line). Made with the
//!   number of lines to come ([`Classifier::expecting`]) it strips no more than that, so output
//!   that starts with `| ` stays output;
//! - reports the second [`PROMPT`] ([`Piece::Evaluated`]): the first asked for the script, so
//!   `mongosh` has evaluated what it read. If that happens before the last line was asked for, the
//!   script ended early (a syntax error, or a `}` that closed the block); and a continuation
//!   prompt past the last line followed by the end of the output means a bracket, string or
//!   comment was left open. Both are [`Classifier::incomplete`];
//! - routes a block that starts (at a line start) with the error marker to stderr until the next
//!   prompt. The marker includes the `\r`, so printed text such as `Uncaught x` stays output.
//!
//! Limitation: once a script redefines `prompt`, its own prompt is shown as output and the end of
//! the script is no longer seen, so from then on the routing to stderr and the outcome are
//! unreliable (an error after it may show as output, and the run may end `Exited(0)`).
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
    /// A continuation prompt: `mongosh` is still reading the statement and wants the next line.
    Continue,
    /// The prompt came back after the script: `mongosh` evaluated what it read.
    Evaluated,
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

/// What text at a line start after a prompt is.
enum Bar<'a> {
    /// A continuation prompt, stripped; the rest.
    Stripped(&'a str),
    /// Could still become one: wait for more.
    Hold,
    Output,
}

#[derive(Debug, Default)]
pub(crate) struct Classifier {
    phase: Phase,
    /// Text not classified yet: an unfinished handshake line, or text that could still become
    /// a prompt, a continuation prompt or an error marker.
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
    /// Continuation prompts still to come (the script lines not asked for yet); `None` when not
    /// known, then every run of them after a prompt is stripped.
    continuations: Option<usize>,
    /// Fixed prompts seen since READY.
    prompts: usize,
    incomplete: bool,
}

impl Classifier {
    /// For a script of `continuations + 1` lines, written a line at a time.
    pub(crate) fn expecting(continuations: usize) -> Self {
        Self {
            continuations: Some(continuations),
            ..Self::default()
        }
    }

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

    /// Whether `mongosh` stopped reading the script before its end (when an error was also seen,
    /// that error is why).
    pub(crate) fn incomplete(&self) -> bool {
        self.incomplete
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
                self.prompt(out);
                rest = after;
                continue;
            }
            if !at_end && PROMPT.starts_with(rest) {
                self.held = rest.to_owned();
                return;
            }
            if self.after_prompt {
                match self.continuation(rest, at_end, out) {
                    Bar::Stripped(after) => {
                        rest = after;
                        continue;
                    }
                    Bar::Hold => {
                        self.held = rest.to_owned();
                        return;
                    }
                    Bar::Output => self.after_prompt = false,
                }
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

    fn prompt(&mut self, out: &mut Vec<Piece>) {
        self.in_error = false;
        self.mid_line = false;
        self.after_prompt = true;
        self.prompts += 1;
        if self.prompts == 2 {
            if self.continuations.is_some_and(|left| left > 0) {
                self.incomplete = true;
            }
            out.push(Piece::Evaluated);
        }
    }

    /// `rest` starts a line right after a prompt (and maybe continuation prompts).
    fn continuation<'a>(&mut self, rest: &'a str, at_end: bool, out: &mut Vec<Piece>) -> Bar<'a> {
        let partial = CONTINUATION.starts_with(rest);
        if self.continuations == Some(0) {
            // Every line was asked for. One more continuation prompt asks for a line after the
            // script's end, and only the end of the output follows it; anything else is output.
            return match (partial, at_end) {
                (true, true) => {
                    self.incomplete = true;
                    Bar::Stripped("")
                }
                (true, false) => Bar::Hold,
                (false, _) => Bar::Output,
            };
        }
        if let Some(after) = rest.strip_prefix(CONTINUATION) {
            if let Some(left) = &mut self.continuations {
                *left -= 1;
            }
            out.push(Piece::Continue);
            return Bar::Stripped(after);
        }
        match partial && !at_end {
            true => Bar::Hold,
            false => Bar::Output,
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
/// A prefix holding another sentinel is not a prompt: `__CB_FAILED__ x> __CB_READY__` failed.
fn after_prompts<'a>(line: &'a str, token: &str) -> Option<&'a str> {
    let at = line.find(token)?;
    let prefix = &line[..at];
    let is_prompts = prefix.is_empty()
        || (prefix.ends_with("> ")
            && !prefix.contains("__CB_")
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
