//! The native open/save dialog for importing and exporting connections. The dialog runs on a
//! worker thread; the answer is polled once per frame. Paths are never logged.
use super::file_picker::{FilePicker, PickPoll, poll_dialog, spawn_dialog};
use curlybrace_core::runtime::Repaint;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

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
        poll_dialog(&self.answers)
    }
}

const FILE_NAME: &str = "curlybrace-connections.json";
const EXTENSION: &str = "json";

pub fn start(picker: Arc<dyn FilePicker>, kind: ConnPickKind, repaint: Repaint) -> ConnPick {
    let answers = spawn_dialog(
        "curlybrace-connection-picker",
        repaint,
        move || match kind {
            ConnPickKind::Import => picker.open_file(EXTENSION),
            ConnPickKind::Export => picker.save_file(FILE_NAME, EXTENSION),
        },
    );
    ConnPick { kind, answers }
}
