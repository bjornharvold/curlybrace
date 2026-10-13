//! The seam to the native "save as" dialog.
//!
//! The reducer never touches a dialog: it only moves the export to `Choosing`. The app layer
//! starts the picker on that edge (`UiEvent::PickExportFile`, emitted by the view together with
//! `CompareAction::ChooseExportFile`, the reducer transition first), waits for the answer
//! without blocking the UI thread, and turns it into `ExportPathChosen` / `ExportPathCancelled`.
//! Paths are never logged.
use curlybrace_core::compare::protocol::ExportFormat;
use curlybrace_core::runtime::Repaint;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use crate::viewmodel::compare::ExportKind;

/// Asks the person where to save a file. Blocking; the app calls it on a worker thread.
pub trait FilePicker: Send + Sync {
    /// `None` when the dialog was cancelled.
    fn save_file(&self, suggested_name: &str, extension: &str) -> Option<PathBuf>;
}

/// The production picker: the operating system's native save dialog (`rfd`).
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeFilePicker;

impl FilePicker for NativeFilePicker {
    fn save_file(&self, suggested_name: &str, extension: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_file_name(suggested_name)
            // The filter label is the extension itself: no language-specific text.
            .add_filter(extension, &[extension])
            .save_file()
    }
}

/// A picker that never opens anything and always answers "cancelled": the safe default for
/// embedding and tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFilePicker;

impl FilePicker for NoFilePicker {
    fn save_file(&self, _suggested_name: &str, _extension: &str) -> Option<PathBuf> {
        None
    }
}

const SCRIPT_NAME: &str = "curlybrace-plan.js";
const SCRIPT_EXT: &str = "js";
const JSON_NAME: &str = "curlybrace-plan.json";
const JSON_EXT: &str = "json";
const REPORT_NAME: &str = "curlybrace-report.csv";
const REPORT_EXT: &str = "csv";

/// The suggested file name and extension for an export (not translated: file names).
/// The format only matters for a plan; a report is always CSV.
pub fn suggested_file(kind: ExportKind, format: ExportFormat) -> (&'static str, &'static str) {
    match (kind, format) {
        (ExportKind::Report, _) => (REPORT_NAME, REPORT_EXT),
        (ExportKind::Plan, ExportFormat::Mongosh) => (SCRIPT_NAME, SCRIPT_EXT),
        (ExportKind::Plan, ExportFormat::JsonPlan) => (JSON_NAME, JSON_EXT),
    }
}

/// What a running dialog has answered so far.
#[derive(Debug, PartialEq, Eq)]
pub enum PickPoll {
    Waiting,
    /// `None`: cancelled (or the worker died, which is treated the same way).
    Done(Option<PathBuf>),
}

/// One dialog in flight.
#[derive(Debug)]
pub struct PendingPick {
    answers: Receiver<Option<PathBuf>>,
}

impl PendingPick {
    pub fn poll(&self) -> PickPoll {
        match self.answers.try_recv() {
            Ok(path) => PickPoll::Done(path),
            Err(TryRecvError::Empty) => PickPoll::Waiting,
            Err(TryRecvError::Disconnected) => PickPoll::Done(None),
        }
    }
}

/// Runs the picker on a worker thread so the UI thread never blocks; wakes the UI afterwards.
pub fn start_pick(
    picker: Arc<dyn FilePicker>,
    suggested_name: &'static str,
    extension: &'static str,
    repaint: Repaint,
) -> PendingPick {
    let (tx, answers) = channel();
    let work = move || {
        let _ = tx.send(picker.save_file(suggested_name, extension));
        repaint();
    };
    // If the thread cannot start the sender is dropped with the closure: reads as cancelled.
    let _ = std::thread::Builder::new()
        .name("curlybrace-file-picker".into())
        .spawn(work);
    PendingPick { answers }
}
