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

/// What the stdout lane tells the writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pace {
    /// Write the next script line.
    Next,
    /// The block was evaluated cleanly: write the epilogue.
    Epilogue,
}

/// What the readers hand the supervisor, in order.
pub(super) enum Delivery {
    Text(Stream, String),
    /// The epilogue's answer, masked.
    Result(String),
}

/// Writes the preamble, then the script a line at a time, each when `pace` asks for it: the first
/// once the shell connected, every other when `mongosh` printed a continuation prompt (it is still
/// reading the same statement). Returning closes stdin, and EOF ends `mongosh`. It returns on a
/// write error (the child exited or never reads), and when `pace` closes: the shell evaluated
/// what it read before the end (a syntax error, or a `}` that closed the block early), did not
/// connect, or its output ended. So the lines after an early evaluation are never sent; sent,
/// `mongosh` would run them one by one.
///
/// After the last line, without an `epilogue` it returns at once. With one it waits for the
/// stdout lane's verdict, which comes when the prompt is back after the block
/// (`Pace::Epilogue`: write it, then return), or as the channel closing: the block was not
/// clean, `mongosh` may be waiting for another line, or the output ended (a cancel, the
/// process exited). Anything else ends it too: it never guesses.
pub(super) fn write_input(
    mut stdin: ChildStdin,
    preamble: Vec<u8>,
    script: Vec<u8>,
    epilogue: Option<Vec<u8>>,
    pace: &Receiver<Pace>,
) {
    let sent = stdin.write_all(&preamble).and_then(|()| stdin.flush());
    // The password's only copy outside the scrubbers: do not keep it for the whole run.
    drop(preamble);
    if sent.is_err() {
        return;
    }
    for line in script.split_inclusive(|&b| b == b'\n') {
        if pace.recv() != Ok(Pace::Next) || stdin.write_all(line).is_err() {
            return;
        }
    }
    let Some(epilogue) = epilogue else {
        return;
    };
    if pace.recv() == Ok(Pace::Epilogue) {
        // A failed write means the shell is gone: there is no answer, and nothing to report.
        let _ = stdin.write_all(&epilogue);
    }
}

/// What a reader does with its text.
pub(super) enum Lane {
    /// Boxed: the classifier is large, and there is one lane of each kind per run.
    Stdout(Box<StdoutLane>),
    Stderr,
}

/// The epilogue's answer a stdout lane looks for.
pub(super) struct Expect<'a> {
    pub(super) token: &'a str,
    pub(super) max_result_bytes: usize,
}

impl Lane {
    /// The stdout lane of a script whose lines after the first are `continuations`; with
    /// `result`, the writer sends an epilogue when this lane says so.
    pub(super) fn stdout(pace: Sender<Pace>, continuations: usize, result: Option<Expect>) -> Self {
        let classifier = match result {
            Some(Expect {
                token,
                max_result_bytes,
            }) => Classifier::with_result(continuations, token, max_result_bytes),
            None => Classifier::expecting(continuations),
        };
        Lane::Stdout(Box::new(StdoutLane {
            classifier,
            pace: Some(pace),
            held_stream: Stream::Stdout,
            result_sent: false,
        }))
    }
}

/// Stdout is classified first and masked after: the masker holds back text that may be the start
/// of a secret, and the prompts that pace the writer must not wait behind it (a password starting
/// with `| ` would stall the script).
pub(super) struct StdoutLane {
    classifier: Classifier,
    /// Asks the writer for the next script line or the epilogue; dropped when no more may be
    /// sent.
    pace: Option<Sender<Pace>>,
    /// The stream of the text the scrubber holds back.
    held_stream: Stream,
    /// An answer was handed on: any later one is dropped.
    result_sent: bool,
}

pub(super) struct Reader {
    pub(super) control: Arc<Control>,
    pub(super) output: Sender<Delivery>,
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
            let allowed = self.admit(n);
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

    /// Counts `n` bytes just read against the budget they belong to: the epilogue's answer window
    /// has its own, so the answer does not count against the text cap. The window opens while the
    /// read that holds the prompt after the block is classified, and the epilogue is written only
    /// then, so its answer always comes in a later read.
    fn admit(&self, n: usize) -> usize {
        match &self.lane {
            Lane::Stdout(lane) if lane.classifier.in_result_window() => {
                self.control.admit_window(n)
            }
            _ => self.control.admit(n),
        }
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
                shown.is_empty()
                    || self
                        .output
                        .send(Delivery::Text(Stream::Stderr, shown))
                        .is_ok()
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
    output: &'a Sender<Delivery>,
}

impl Sink<'_> {
    fn text(&self, stream: Stream, text: String) -> bool {
        self.output.send(Delivery::Text(stream, text)).is_ok()
    }
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
            Piece::Evaluated { epilogue } => {
                if epilogue {
                    self.send_pace(Pace::Epilogue);
                }
                // No more lines, and no epilogue unless just asked for.
                self.pace.take();
                true
            }
            Piece::NoEpilogue => {
                self.pace.take();
                true
            }
            Piece::Result(payload) => self.result(&payload, sink),
            Piece::ResultTooLarge => {
                log::warn!("shell: the structured result was over the size cap and was dropped");
                true
            }
            Piece::Failed(message) => {
                sink.control.mark_handshake_done();
                self.pace.take();
                let shown = sink.scrubber.mask(&format!("{message}\n"));
                let _ = sink.text(Stream::Stderr, shown);
                sink.control.stop(Stop::ConnectFailed);
                false
            }
            Piece::NoSentinel(lines) => {
                self.pace.take();
                // They explain a shell that never connected; after a cancel they are noise.
                if !lines.is_empty() && !sink.control.cancelled() {
                    let _ = sink.text(Stream::Stderr, sink.scrubber.mask(&lines));
                }
                sink.control.stop(Stop::ConnectFailed);
                false
            }
        }
    }

    /// Hands the first answer on, masked as a whole (the classifier kept the line whole, so a
    /// secret split across reads is masked too); a later one, or one after a stop, is dropped.
    fn result(&mut self, payload: &str, sink: &mut Sink<'_>) -> bool {
        if self.result_sent || sink.control.stopped() {
            return true;
        }
        self.result_sent = true;
        let masked = sink.scrubber.mask(payload);
        sink.output.send(Delivery::Result(masked)).is_ok()
    }

    fn next_line(&self) {
        self.send_pace(Pace::Next);
    }

    fn send_pace(&self, pace: Pace) {
        if let Some(sender) = &self.pace {
            // The writer may be gone (a write failed): nothing more to send then.
            let _ = sender.send(pace);
        }
    }

    /// Masks and sends `text`. The held tail belongs to the stream it came from, so it is released
    /// there when the stream changes: that happens only at a line start, which no secret spans.
    fn out(&mut self, stream: Stream, text: &str, sink: &mut Sink<'_>) -> bool {
        if stream != self.held_stream {
            let tail = sink.scrubber.finish();
            if !tail.is_empty() && !sink.text(self.held_stream, tail) {
                return false;
            }
            self.held_stream = stream;
        }
        let shown = sink.scrubber.push(text);
        shown.is_empty() || sink.text(stream, shown)
    }

    /// Releases the held tail at the end of a run that was not stopped.
    fn release(&mut self, sink: &mut Sink<'_>) {
        let tail = sink.scrubber.finish();
        if !tail.is_empty() {
            let _ = sink.text(self.held_stream, tail);
        }
    }
}
