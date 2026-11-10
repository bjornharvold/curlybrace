//! Turns `mongosh` stdout (already decoded and scrubbed) into what the Shell tab shows.
//!
//! `mongosh --nodb --quiet` reading piped stdin behaves like its REPL (see
//! `docs/superpowers/notes/shell-stdin-spike.md`): it prints a prompt (`> `, or `<db>> ` once
//! connected) before each line it reads, and errors on stdout as `Uncaught \r<Error>...`.
//! The classifier
//! - hides everything up to the preamble's `__CB_READY__` / `__CB_FAILED__ <message>` line,
//!   plus the blank line `print` leaves after it;
//! - strips prompts (`^[A-Za-z0-9_.-]*> ` at a line start; and at the very end of the output, a
//!   bare `> ` or the last line-start prompt after unterminated text) and every `\r`;
//! - routes a block starting with a line that begins `Uncaught` to stderr until the next prompt.
//!   A syntax error's code frame (`> 1 | code`) inside such a block is not a prompt.
use super::Stream;

const READY: &str = "__CB_READY__";
const FAILED: &str = "__CB_FAILED__";
const UNCAUGHT: &str = "Uncaught";
/// Longest prompt name recognised (MongoDB database names are shorter). A longer run of name
/// characters at a line start is output, which keeps a long unterminated line streaming.
const PROMPT_NAME_MAX: usize = 64;
const BARE_PROMPT: &str = "> ";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Piece {
    Out(Stream, String),
    /// The preamble connected; the script may be written.
    Ready,
    /// The preamble failed with this message; nothing after it is shown.
    Failed(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Handshake,
    Running,
    Failed,
}

/// What a line start holds.
enum LineStart {
    /// A prompt of this many bytes.
    Prompt(usize),
    /// Not decidable yet: could still become a prompt (or the start of `Uncaught`).
    NeedMore,
    NotPrompt,
}

#[derive(Debug, Default)]
pub(crate) struct Classifier {
    phase: Phase,
    /// Text not classified yet: an unfinished handshake line, a line start that could still be a
    /// prompt, or the end of a line that could still become a trailing prompt.
    held: String,
    /// The last prompt seen at a line start (`admin> `); empty before the first one.
    prompt: String,
    /// False while `held` (or the next text) starts a line.
    mid_line: bool,
    in_error: bool,
    /// Drop one `\n` right after the READY line (the empty result `print` leaves).
    skip_blank: bool,
    saw_uncaught: bool,
}

impl Classifier {
    pub(crate) fn push(&mut self, text: &str) -> Vec<Piece> {
        self.feed(&text.replace('\r', ""), false)
    }

    /// Releases what is held; call once when stdout ends.
    pub(crate) fn finish(&mut self) -> Vec<Piece> {
        self.feed("", true)
    }

    /// Whether any `Uncaught` error block was seen.
    pub(crate) fn saw_uncaught(&self) -> bool {
        self.saw_uncaught
    }

    fn feed(&mut self, text: &str, at_end: bool) -> Vec<Piece> {
        let mut out = Vec::new();
        match self.phase {
            Phase::Handshake => self.handshake(text, at_end, &mut out),
            Phase::Running => self.run(text, at_end, &mut out),
            Phase::Failed => {}
        }
        out
    }

    fn handshake(&mut self, text: &str, at_end: bool, out: &mut Vec<Piece>) {
        self.held.push_str(text);
        if at_end && !self.held.is_empty() && !self.held.ends_with('\n') {
            self.held.push('\n');
        }
        while let Some(nl) = self.held.find('\n') {
            let line = self.held[..nl].to_owned();
            let rest = self.held[nl + 1..].to_owned();
            let content = strip_prompts(&line);
            if content == READY {
                self.phase = Phase::Running;
                self.held.clear();
                self.skip_blank = true;
                out.push(Piece::Ready);
                self.run(&rest, at_end, out);
                return;
            }
            if let Some(message) = content.strip_prefix(FAILED) {
                if message.is_empty() || message.starts_with(' ') {
                    self.phase = Phase::Failed;
                    self.held.clear();
                    out.push(Piece::Failed(message.trim_start().to_owned()));
                    return;
                }
            }
            self.held = rest;
        }
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
            if !self.mid_line {
                match self.line_start(rest, at_end) {
                    LineStart::Prompt(len) => {
                        self.in_error = false;
                        self.prompt = rest[..len].to_owned();
                        rest = &rest[len..];
                        continue;
                    }
                    LineStart::NeedMore => {
                        self.held = rest.to_owned();
                        return;
                    }
                    LineStart::NotPrompt => {
                        if rest.starts_with(UNCAUGHT) {
                            self.in_error = true;
                            self.saw_uncaught = true;
                        }
                        self.mid_line = true;
                    }
                }
            }
            let stream = if self.in_error {
                Stream::Stderr
            } else {
                Stream::Stdout
            };
            if let Some(nl) = rest.find('\n') {
                emit(out, stream, &rest[..=nl]);
                rest = &rest[nl + 1..];
                self.mid_line = false;
                continue;
            }
            let keep = self.trailing_prompt_len(rest);
            let (shown, tail) = rest.split_at(rest.len() - keep);
            emit(out, stream, shown);
            if !at_end {
                self.held = tail.to_owned();
            } else if tail != BARE_PROMPT && tail != self.prompt {
                emit(out, stream, tail);
            }
            return;
        }
    }

    /// Length of the longest end of `text` that could still become a trailing prompt.
    fn trailing_prompt_len(&self, text: &str) -> usize {
        [BARE_PROMPT, self.prompt.as_str()]
            .iter()
            .flat_map(|prompt| {
                (1..=prompt.len())
                    .filter(|k| prompt.is_char_boundary(*k))
                    .map(|k| &prompt[..k])
            })
            .filter(|start| text.ends_with(start))
            .map(str::len)
            .max()
            .unwrap_or(0)
    }

    fn line_start(&self, rest: &str, at_end: bool) -> LineStart {
        let name = rest.bytes().take_while(|b| is_name_byte(*b)).count();
        if name > PROMPT_NAME_MAX {
            return LineStart::NotPrompt;
        }
        let after = &rest[name..];
        let Some(after) = after.strip_prefix('>') else {
            return if after.is_empty() && !at_end {
                LineStart::NeedMore
            } else {
                LineStart::NotPrompt
            };
        };
        let Some(after) = after.strip_prefix(' ') else {
            return if after.is_empty() && !at_end {
                LineStart::NeedMore
            } else {
                LineStart::NotPrompt
            };
        };
        if self.in_error && name == 0 {
            match code_frame(after) {
                Some(true) => return LineStart::NotPrompt,
                None if !at_end => return LineStart::NeedMore,
                _ => {}
            }
        }
        LineStart::Prompt(name + 2)
    }
}

/// `line` without its leading prompts.
fn strip_prompts(line: &str) -> &str {
    let mut rest = line;
    loop {
        let name = rest.bytes().take_while(|b| is_name_byte(*b)).count();
        match rest[name..].strip_prefix("> ") {
            Some(after) if name <= PROMPT_NAME_MAX => rest = after,
            _ => return rest,
        }
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')
}

/// Whether text after `> ` is a code-frame line (`<digits> |`); `None` while undecided.
fn code_frame(after: &str) -> Option<bool> {
    let digits = after.bytes().take_while(u8::is_ascii_digit).count();
    let tail = &after[digits..];
    match (digits, tail) {
        (_, "") | (1.., " ") => None,
        (0, _) => Some(false),
        _ => Some(tail.starts_with(" |")),
    }
}

/// Appends text, merging with the previous piece when it is on the same stream.
fn emit(out: &mut Vec<Piece>, stream: Stream, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Piece::Out(last, existing)) = out.last_mut() {
        if *last == stream {
            existing.push_str(text);
            return;
        }
    }
    out.push(Piece::Out(stream, text.to_owned()));
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
