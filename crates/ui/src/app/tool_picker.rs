//! The system file dialog for choosing a tool's executable. The dialog runs on a worker thread;
//! the answer is polled once per frame. Paths are never logged.
use super::file_picker::{FilePicker, PickPoll};
use curlybrace_core::runtime::Repaint;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// One dialog in flight.
#[derive(Debug)]
pub(super) struct ToolPick {
    answers: Receiver<Option<PathBuf>>,
}

impl ToolPick {
    pub(super) fn poll(&self) -> PickPoll {
        match self.answers.try_recv() {
            Ok(path) => PickPoll::Done(path),
            Err(TryRecvError::Empty) => PickPoll::Waiting,
            Err(TryRecvError::Disconnected) => PickPoll::Done(None),
        }
    }
}

pub(super) fn start(picker: Arc<dyn FilePicker>, repaint: Repaint) -> ToolPick {
    let (tx, answers) = channel();
    let work = move || {
        let _ = tx.send(picker.pick_executable());
        repaint();
    };
    // If the thread cannot start the sender is dropped with the closure: reads as cancelled.
    let _ = std::thread::Builder::new()
        .name("curlybrace-tool-picker".into())
        .spawn(work);
    ToolPick { answers }
}
