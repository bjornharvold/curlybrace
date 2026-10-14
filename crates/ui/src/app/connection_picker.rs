//! The native open/save dialog for importing and exporting connections. The dialog runs on a
//! worker thread; the answer is polled once per frame. Paths are never logged.
use super::file_picker::{FilePicker, PickPoll};
use curlybrace_core::runtime::Repaint;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnPickKind {
    Import,
    Export,
}

#[derive(Debug)]
pub struct ConnPick {
    pub kind: ConnPickKind,
    answers: Receiver<Option<PathBuf>>,
}

impl ConnPick {
    pub fn poll(&self) -> PickPoll {
        match self.answers.try_recv() {
            Ok(path) => PickPoll::Done(path),
            Err(TryRecvError::Empty) => PickPoll::Waiting,
            Err(TryRecvError::Disconnected) => PickPoll::Done(None),
        }
    }
}

const FILE_NAME: &str = "curlybrace-connections.json";
const EXTENSION: &str = "json";

pub fn start(picker: Arc<dyn FilePicker>, kind: ConnPickKind, repaint: Repaint) -> ConnPick {
    let (tx, answers) = channel();
    let work = move || {
        let chosen = match kind {
            ConnPickKind::Import => picker.open_file(EXTENSION),
            ConnPickKind::Export => picker.save_file(FILE_NAME, EXTENSION),
        };
        let _ = tx.send(chosen);
        repaint();
    };
    // If the thread cannot start the sender is dropped with the closure: reads as cancelled.
    let _ = std::thread::Builder::new()
        .name("curlybrace-connection-picker".into())
        .spawn(work);
    ConnPick { kind, answers }
}
