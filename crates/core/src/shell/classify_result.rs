//! The classifier's epilogue part: the verdict on sending it, and lifting its answer out of the
//! text.
//!
//! The epilogue may be sent only once `mongosh` evaluated the whole block: the second prompt
//! came (`Piece::Evaluated`), every script line was asked for, no error block was seen, and
//! `mongosh` did not say that the script assigned `_` (then `_` is the script's own value, not
//! the block's). The verdict is given once, with `Evaluated`; a continuation prompt after the
//! last line gives it early as `NoEpilogue` (`mongosh` may be waiting for input that never
//! comes, so the writer must close stdin now, as it always did).
//!
//! After `Evaluated { epilogue: true }` and up to the next prompt (the epilogue's own), a line
//! that starts with the marker (U+0001, `CBJ-`, the run's token, a space) is the answer: it is
//! never text, the single empty line `print` leaves after it is dropped, and an answer over the
//! cap is dropped (`ResultTooLarge`) without buffering more of it. Every other line in that
//! window (late output of the script's timers, say) is ordinary text. The script ran before the
//! token was sent, so a marker line before the window cannot be an answer: it is text.
use super::{Classifier, Piece};

/// What `mongosh` prints when a script assigns `_`: from then on `_` keeps that value.
const UNDERSCORE_NOTICE: &str = "Expression assignment to _ now disabled.";

/// What text at a line start in the answer window is.
pub(super) enum Start<'a> {
    /// The marker, stripped: an answer follows.
    Answer(&'a str),
    /// Could still become the marker: wait for more.
    Hold,
    Other,
}

/// The answer line read so far. Its `Debug` shows the length only.
#[derive(Default, PartialEq, Eq)]
pub(super) struct PayloadLine {
    text: String,
    /// Went over the cap: the rest of the line is dropped.
    over: bool,
}

impl std::fmt::Debug for PayloadLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PayloadLine(<{} bytes>, over: {})",
            self.text.len(),
            self.over
        )
    }
}

impl Classifier {
    /// Whether the epilogue may be sent; asked once, at the second prompt.
    pub(super) fn epilogue_wanted(&self) -> bool {
        self.marker.is_some()
            && !self.verdict_given
            && self.continuations == Some(0)
            && !self.incomplete
            && !self.saw_uncaught
            && !self.saw_underscore
    }

    /// Gives the verdict "no epilogue" now, unless it was given.
    pub(super) fn refuse_epilogue(&mut self, out: &mut Vec<Piece>) {
        if self.marker.is_some() && !self.verdict_given && self.prompts < 2 {
            self.verdict_given = true;
            out.push(Piece::NoEpilogue);
        }
    }

    /// Whether `rest` (at a line start) starts an answer.
    pub(super) fn result_start<'a>(&self, rest: &'a str, at_end: bool) -> Start<'a> {
        let marker = match (&self.marker, self.window) {
            (Some(marker), true) => marker.as_str(),
            _ => return Start::Other,
        };
        if let Some(after) = rest.strip_prefix(marker) {
            return Start::Answer(after);
        }
        match !at_end && marker.starts_with(rest) {
            true => Start::Hold,
            false => Start::Other,
        }
    }

    /// Takes the answer's text from `rest` up to its line end; returns what is left.
    pub(super) fn payload_text<'a>(&mut self, rest: &'a str, out: &mut Vec<Piece>) -> &'a str {
        let (part, left, ended) = match rest.find('\n') {
            Some(i) => (&rest[..i], &rest[i + 1..], true),
            None => (rest, "", false),
        };
        let max = self.max_result_bytes;
        let Some(line) = self.payload.as_mut() else {
            return rest;
        };
        if line.push(part, max) {
            out.push(Piece::ResultTooLarge);
        }
        if ended {
            if let Some(line) = self.payload.take() {
                if !line.over {
                    out.push(Piece::Result(line.text.replace('\r', "")));
                }
            }
            self.skip_result_blank = true;
            self.mid_line = false;
        }
        left
    }

    /// Looks for the `_` notice in the block's output, also when split across pieces.
    pub(super) fn note_block_output(&mut self, segment: &str) {
        if self.saw_underscore {
            return;
        }
        let joined = std::mem::take(&mut self.block_tail) + segment;
        if joined.contains(UNDERSCORE_NOTICE) {
            self.saw_underscore = true;
            return;
        }
        let mut cut = joined.len().saturating_sub(UNDERSCORE_NOTICE.len() - 1);
        while !joined.is_char_boundary(cut) {
            cut += 1;
        }
        self.block_tail = joined[cut..].to_owned();
    }

    /// Bytes of the answer held so far.
    #[cfg(test)]
    pub(super) fn result_buffered(&self) -> usize {
        self.payload.as_ref().map_or(0, |line| line.text.len())
    }
}

impl PayloadLine {
    /// Adds `part`, unless that makes the line longer than `max`; true when this push went over.
    fn push(&mut self, part: &str, max: usize) -> bool {
        if self.over {
            return false;
        }
        if self.text.len() + part.len() > max {
            self.over = true;
            self.text = String::new();
            return true;
        }
        self.text.push_str(part);
        false
    }
}
