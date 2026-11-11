//! The threads on the child's pipes: the stdin writer and one reader per output stream.
use super::Scrubber;
use super::classify::{Classifier, Piece};
use super::control::{Control, Stop};
use super::decode::Utf8Decoder;
use super::run::Stream;
use std::io::{ErrorKind, Read, Write};
use std::process::ChildStdin;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

/// Writes the preamble, then the script a line at a time, each when `pace` asks for it: the first
/// once the shell connected, every other when `mongosh` printed a continuation prompt (it is still
/// reading the same statement). Returning closes stdin, and EOF ends `mongosh`. It returns after
/// the last line, on a write error (the child exited or never reads), and when `pace` closes:
/// the shell evaluated what it read before the end (a syntax error, or a `}` that closed the
/// block early), did not connect, or its output ended. So the lines after an early evaluation
/// are never sent; sent, `mongosh` would run them one by one.
pub(super) fn write_input(
    mut stdin: ChildStdin,
    preamble: Vec<u8>,
    script: Vec<u8>,
    pace: &Receiver<()>,
) {
    let sent = stdin.write_all(&preamble).and_then(|()| stdin.flush());
    // The password's only copy outside the scrubbers: do not keep it for the whole run.
    drop(preamble);
    if sent.is_err() {
        return;
    }
    for line in script.split_inclusive(|&b| b == b'\n') {
        if pace.recv().is_err() || stdin.write_all(line).is_err() {
            return;
        }
    }
}

/// What a reader does with its text.
pub(super) enum Lane {
    Stdout(StdoutLane),
    Stderr,
}

impl Lane {
    /// The stdout lane of a script whose lines after the first are `continuations`.
    pub(super) fn stdout(pace: Sender<()>, continuations: usize) -> Self {
        Lane::Stdout(StdoutLane {
            classifier: Classifier::expecting(continuations),
            pace: Some(pace),
            held_stream: Stream::Stdout,
        })
    }
}

/// Stdout is classified first and masked after: the masker holds back text that may be the start
/// of a secret, and the prompts that pace the writer must not wait behind it (a password starting
/// with `| ` would stall the script).
pub(super) struct StdoutLane {
    classifier: Classifier,
    /// Asks the writer for the next script line; dropped when no more may be sent.
    pace: Option<Sender<()>>,
    /// The stream of the text the scrubber holds back.
    held_stream: Stream,
}

pub(super) struct Reader {
    pub(super) control: Arc<Control>,
    pub(super) output: Sender<(Stream, String)>,
    pub(super) scrubber: Scrubber,
    pub(super) decoder: Utf8Decoder,
    pub(super) lane: Lane,
}

impl Reader {
    pub(super) fn run(mut self, mut source: impl Read, chunk_bytes: usize) {
        let mut buf = vec![0; chunk_bytes];
        loop {
            let n = match source.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let allowed = self.control.admit(n);
            let text = self.decoder.push(&buf[..allowed]);
            if !self.pass(&text, false) || allowed < n {
                break;
            }
        }
        // When the run was stopped the output is cut: the held tail may be the start of a secret
        // whose rest never comes, so it is dropped, not released.
        let text = match self.control.stopped() {
            true => String::new(),
            false => self.decoder.finish(),
        };
        self.pass(&text, true);
    }

    /// Sends what `text` shows; false once nothing more should be read.
    fn pass(&mut self, text: &str, at_end: bool) -> bool {
        let release = at_end && !self.control.stopped();
        match &mut self.lane {
            Lane::Stderr => {
                let mut shown = self.scrubber.push(text);
                if release {
                    shown.push_str(&self.scrubber.finish());
                }
                let shown = shown.replace('\r', "");
                shown.is_empty() || self.output.send((Stream::Stderr, shown)).is_ok()
            }
            Lane::Stdout(lane) => {
                let mut pieces = lane.classifier.push(text);
                if at_end {
                    pieces.extend(lane.classifier.finish());
                }
                if lane.classifier.saw_uncaught() {
                    self.control.mark_uncaught();
                }
                if lane.classifier.incomplete() {
                    self.control.mark_incomplete();
                }
                let mut sink = Sink {
                    scrubber: &mut self.scrubber,
                    control: &self.control,
                    output: &self.output,
                };
                let more = pieces.into_iter().all(|piece| lane.piece(piece, &mut sink));
                if release {
                    lane.release(&mut sink);
                }
                more
            }
        }
    }
}

/// Where the stdout lane's pieces go.
struct Sink<'a> {
    scrubber: &'a mut Scrubber,
    control: &'a Control,
    output: &'a Sender<(Stream, String)>,
}

impl StdoutLane {
    /// Acts on one classified piece; false once nothing more should be read.
    fn piece(&mut self, piece: Piece, sink: &mut Sink<'_>) -> bool {
        match piece {
            Piece::Out(stream, text) => self.out(stream, &text, sink),
            Piece::Ready => {
                sink.control.mark_handshake_done();
                self.next_line();
                true
            }
            Piece::Continue => {
                self.next_line();
                true
            }
            Piece::Evaluated => {
                self.pace.take();
                true
            }
            Piece::Failed(message) => {
                sink.control.mark_handshake_done();
                self.pace.take();
                let shown = sink.scrubber.mask(&format!("{message}\n"));
                let _ = sink.output.send((Stream::Stderr, shown));
                sink.control.stop(Stop::ConnectFailed);
                false
            }
            Piece::NoSentinel(lines) => {
                self.pace.take();
                // They explain a shell that never connected; after a cancel they are noise.
                if !lines.is_empty() && !sink.control.cancelled() {
                    let _ = sink
                        .output
                        .send((Stream::Stderr, sink.scrubber.mask(&lines)));
                }
                sink.control.stop(Stop::ConnectFailed);
                false
            }
        }
    }

    fn next_line(&self) {
        if let Some(pace) = &self.pace {
            let _ = pace.send(());
        }
    }

    /// Masks and sends `text`. The held tail belongs to the stream it came from, so it is released
    /// there when the stream changes: that happens only at a line start, which no secret spans.
    fn out(&mut self, stream: Stream, text: &str, sink: &mut Sink<'_>) -> bool {
        if stream != self.held_stream {
            let tail = sink.scrubber.finish();
            if !tail.is_empty() && sink.output.send((self.held_stream, tail)).is_err() {
                return false;
            }
            self.held_stream = stream;
        }
        let shown = sink.scrubber.push(text);
        shown.is_empty() || sink.output.send((stream, shown)).is_ok()
    }

    /// Releases the held tail at the end of a run that was not stopped.
    fn release(&mut self, sink: &mut Sink<'_>) {
        let tail = sink.scrubber.finish();
        if !tail.is_empty() {
            let _ = sink.output.send((self.held_stream, tail));
        }
    }
}
