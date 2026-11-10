//! The Shell tab's captured output: a persistent, structurally shared log of lines.
//!
//! Lines live in segments of `SEGMENT_LINES` behind `Arc`s, and each line is itself an `Arc`, so
//! cloning a log copies only the spine and appending copies the spine, the last segment's `Arc`s
//! and the one line being continued. Old states keep their lines. Pure and egui-free.
use curlybrace_core::shell::Stream;
use std::collections::VecDeque;
use std::sync::Arc;

/// The most lines kept; older lines are dropped first.
pub const MAX_OUTPUT_LINES: usize = 10_000;
/// The most bytes kept of one line; the rest of an over-long line is discarded.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// Lines per segment.
const SEGMENT_LINES: usize = 256;

/// One line of output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLine {
    pub stream: Stream,
    pub text: String,
    /// True when bytes beyond `MAX_LINE_BYTES` were discarded.
    pub truncated: bool,
}

type Segment = Arc<Vec<Arc<OutputLine>>>;

#[derive(Clone, Debug, Default)]
pub struct OutputLog {
    segments: VecDeque<Segment>,
    /// Lines of the first segment already dropped.
    skip: usize,
    len: usize,
    /// The last line has not seen its newline, so the next chunk of its stream continues it.
    open: bool,
}

impl PartialEq for OutputLog {
    fn eq(&self, other: &Self) -> bool {
        self.open == other.open && self.len == other.len && self.iter().eq(other.iter())
    }
}

impl OutputLog {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The line at `index`, 0 being the oldest retained line.
    pub fn get(&self, index: usize) -> Option<&OutputLine> {
        if index >= self.len {
            return None;
        }
        let at = self.skip + index;
        self.segments
            .get(at / SEGMENT_LINES)
            .and_then(|seg| seg.get(at % SEGMENT_LINES))
            .map(|line| &**line)
    }

    pub fn iter(&self) -> impl Iterator<Item = &OutputLine> {
        (0..self.len).filter_map(|i| self.get(i))
    }

    /// The log with its last line no longer continuable.
    pub fn closed(&self) -> Self {
        Self {
            open: false,
            ..self.clone()
        }
    }

    /// The log after `text` of `stream`, and how many oldest lines the window dropped.
    pub fn appended(&self, stream: Stream, text: &str) -> (Self, u64) {
        let mut next = self.clone();
        for piece in text.split_inclusive('\n') {
            let closed = piece.ends_with('\n');
            let body = piece.strip_suffix('\n').unwrap_or(piece);
            next.add_piece(stream, body, closed);
        }
        let dropped = next.trim_to_window();
        (next, dropped)
    }

    fn add_piece(&mut self, stream: Stream, body: &str, closed: bool) {
        let continues = self.open && self.last_mut().is_some_and(|l| l.stream == stream);
        if !continues {
            self.push_line(OutputLine {
                stream,
                text: String::new(),
                truncated: false,
            });
        }
        if let Some(line) = self.last_mut() {
            let room = MAX_LINE_BYTES.saturating_sub(line.text.len());
            let mut cut = body.len().min(room);
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            line.text.push_str(&body[..cut]);
            line.truncated |= cut < body.len();
            if closed && line.text.ends_with('\r') {
                line.text.pop();
            }
        }
        self.open = !closed;
    }

    fn last_mut(&mut self) -> Option<&mut OutputLine> {
        let seg = self.segments.back_mut()?;
        let line = Arc::make_mut(seg).last_mut()?;
        Some(Arc::make_mut(line))
    }

    fn push_line(&mut self, line: OutputLine) {
        if self
            .segments
            .back()
            .is_none_or(|s| s.len() >= SEGMENT_LINES)
        {
            self.segments.push_back(Arc::new(Vec::new()));
        }
        if let Some(seg) = self.segments.back_mut() {
            Arc::make_mut(seg).push(Arc::new(line));
            self.len += 1;
        }
    }

    /// Drops the oldest lines beyond the window by advancing `skip` and popping whole segments.
    fn trim_to_window(&mut self) -> u64 {
        let excess = self.len.saturating_sub(MAX_OUTPUT_LINES);
        if excess == 0 {
            return 0;
        }
        let total = self.skip + excess;
        for _ in 0..total / SEGMENT_LINES {
            self.segments.pop_front();
        }
        self.skip = total % SEGMENT_LINES;
        self.len -= excess;
        excess as u64
    }

    #[cfg(test)]
    pub(crate) fn first_segment_shared_with(&self, other: &Self) -> bool {
        match (self.segments.front(), other.segments.front()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "shell_output_tests.rs"]
mod tests;
