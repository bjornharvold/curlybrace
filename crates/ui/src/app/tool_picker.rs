//! The system file dialog for choosing a tool's executable. It runs on a worker thread (see
//! `file_picker::spawn_dialog`); the answer is polled once per frame. Paths are never logged.
use super::file_picker::{FilePicker, PickPoll, poll_dialog, spawn_dialog};
use super::tools_state::ProbeScope;
use curlybrace_core::runtime::Repaint;
use curlybrace_core::tools::ToolKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// One dialog in flight, and the tool it was opened for: an answer is only used while that is
/// still the tool the state is waiting for.
#[derive(Debug)]
pub(super) struct ToolPick {
    target: (ProbeScope, ToolKind),
    answers: Receiver<Option<PathBuf>>,
}

impl ToolPick {
    pub(super) fn target(&self) -> (ProbeScope, ToolKind) {
        self.target
    }

    pub(super) fn poll(&self) -> PickPoll {
        poll_dialog(&self.answers)
    }
}

pub(super) fn start(
    picker: Arc<dyn FilePicker>,
    target: (ProbeScope, ToolKind),
    repaint: Repaint,
) -> ToolPick {
    let answers = spawn_dialog("curlybrace-tool-picker", repaint, move || {
        picker.pick_executable()
    });
    ToolPick { target, answers }
}
