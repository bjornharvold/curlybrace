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

/// Writes the preamble, then the script once `ready` says the shell connected, then closes
/// stdin (EOF ends `mongosh`). Any write error (the child exited or never reads) just ends it;
/// so does `ready` closing without a message (no connect).
pub(super) fn write_input(
    mut stdin: ChildStdin,
    preamble: Vec<u8>,
    script: Vec<u8>,
    ready: &Receiver<()>,
) {
    let sent = stdin.write_all(&preamble).and_then(|()| stdin.flush());
    // The password's only copy outside the scrubbers: do not keep it for the whole run.
    drop(preamble);
    if sent.is_ok() && ready.recv().is_ok() {
        let _ = stdin.write_all(&script);
    }
}

/// What a reader does with scrubbed text.
pub(super) enum Lane {
    Stdout {
        classifier: Classifier,
        /// Tells the writer the shell connected; dropped unsent when it did not.
        ready: Option<Sender<()>>,
    },
    Stderr,
}

impl Lane {
    pub(super) fn stdout(ready: Sender<()>) -> Self {
        Lane::Stdout {
            classifier: Classifier::default(),
            ready: Some(ready),
        }
    }
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
            let text = self.scrubber.push(&self.decoder.push(&buf[..allowed]));
            if !self.pass(&text, false) || allowed < n {
                break;
            }
        }
        // When the run was stopped the output is cut: the scrubber's held tail may be the start
        // of a secret whose rest never comes, so it is dropped, not released.
        let text = if self.control.stopped() {
            String::new()
        } else {
            let mut text = self.scrubber.push(&self.decoder.finish());
            text.push_str(&self.scrubber.finish());
            text
        };
        self.pass(&text, true);
    }

    /// Sends what `text` shows; false once nothing more should be read.
    fn pass(&mut self, text: &str, at_end: bool) -> bool {
        match &mut self.lane {
            Lane::Stderr => {
                let text = text.replace('\r', "");
                text.is_empty() || self.output.send((Stream::Stderr, text)).is_ok()
            }
            Lane::Stdout { classifier, ready } => {
                let mut pieces = classifier.push(text);
                if at_end {
                    pieces.extend(classifier.finish());
                }
                if classifier.saw_uncaught() {
                    self.control.mark_uncaught();
                }
                pieces
                    .into_iter()
                    .all(|piece| stdout_piece(piece, ready, &self.control, &self.output))
            }
        }
    }
}

/// Acts on one classified stdout piece; false once nothing more should be read.
fn stdout_piece(
    piece: Piece,
    ready: &mut Option<Sender<()>>,
    control: &Control,
    output: &Sender<(Stream, String)>,
) -> bool {
    match piece {
        Piece::Out(stream, text) => output.send((stream, text)).is_ok(),
        Piece::Ready => {
            control.mark_handshake_done();
            if let Some(ready) = ready.take() {
                let _ = ready.send(());
            }
            true
        }
        Piece::Failed(message) => {
            control.mark_handshake_done();
            ready.take();
            let _ = output.send((Stream::Stderr, format!("{message}\n")));
            control.stop(Stop::ConnectFailed);
            false
        }
        Piece::NoSentinel(lines) => {
            ready.take();
            if !lines.is_empty() {
                let _ = output.send((Stream::Stderr, lines));
            }
            control.stop(Stop::ConnectFailed);
            false
        }
    }
}
