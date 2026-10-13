//! Gestures and a fake file dialog for the export flows. The native dialog is never opened:
//! the app asks a [`FakePicker`], which answers with a path inside the test's temp directory.
use super::actions::{click_key, wait_shows};
use super::fixture::Fixture;
use super::wait::{SHORT, settle, wait_until};
use curlybrace_ui::app::file_picker::FilePicker;
use curlybrace_ui::viewmodel::compare::{ExportKind, ExportUi};
use eframe::egui::OutputCommand;
use egui_kittest::kittest::Queryable;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Answers every save dialog with the next queued path (None = the person cancelled).
#[derive(Default)]
pub struct FakePicker {
    answers: Mutex<Vec<Option<PathBuf>>>,
    asked: Mutex<Vec<(String, String)>>,
}

impl FakePicker {
    pub fn answering(paths: Vec<PathBuf>) -> Arc<Self> {
        let answers = paths.into_iter().rev().map(Some).collect();
        Arc::new(Self {
            answers: Mutex::new(answers),
            asked: Mutex::new(Vec::new()),
        })
    }

    /// The `(suggested name, extension)` of each dialog so far.
    pub fn asked(&self) -> Vec<(String, String)> {
        self.asked.lock().unwrap().clone()
    }
}

impl FilePicker for FakePicker {
    fn save_file(&self, suggested_name: &str, extension: &str) -> Option<PathBuf> {
        self.asked
            .lock()
            .unwrap()
            .push((suggested_name.to_string(), extension.to_string()));
        self.answers.lock().unwrap().pop().flatten()
    }
}

/// Clicks the entry button (`compare.export.open_plan` / `open_report`) and waits for the modal.
#[track_caller]
pub fn open_export(f: &mut Fixture, kind: ExportKind) {
    let (button, title) = match kind {
        ExportKind::Plan => ("compare.export.open_plan", "compare.export.title.plan"),
        ExportKind::Report => ("compare.export.open_report", "compare.export.title.report"),
    };
    click_key(f, button);
    let title = f.labels.t(title);
    wait_shows(f, &title);
}

/// Clicks "Choose file and export" and waits until the export has finished (bounded).
#[track_caller]
pub fn choose_file_and_wait(f: &mut Fixture) {
    click_key(f, "compare.export.choose_file");
    wait_until(&mut f.harness, "the export to finish", SHORT, |h| {
        matches!(h.state().state().compare.export, ExportUi::Finished { .. })
    });
    let title = f.labels.t("compare.export.finished.title");
    wait_shows(f, &title);
}

/// The finished outcome, once there is one.
#[track_caller]
pub fn finished(f: &Fixture) -> curlybrace_core::compare::protocol::ExportOutcome {
    match &f.harness.state().state().compare.export {
        ExportUi::Finished { outcome, .. } => *outcome,
        other => panic!("not finished: {other:?}"),
    }
}

pub fn dismiss(f: &mut Fixture) {
    click_key(f, "compare.export.dismiss");
}

/// Clicks "Copy to clipboard" and steps frames until the report was handed over; returns every
/// text the app copied to the clipboard meanwhile.
#[track_caller]
pub fn copy_report(f: &mut Fixture) -> Vec<String> {
    let label = f.labels.t("compare.export.copy");
    f.harness.get_by_label(&label).click();
    let mut copied = Vec::new();
    let mut collect = |h: &egui_kittest::Harness<'_, curlybrace_ui::app::CurlybraceApp>| {
        for cmd in &h.output().platform_output.commands {
            if let OutputCommand::CopyText(text) = cmd {
                copied.push(text.clone());
            }
        }
    };
    wait_until(&mut f.harness, "the report to be copied", SHORT, |h| {
        collect(h);
        matches!(
            h.state().state().compare.export,
            ExportUi::Copied { text: None, .. }
        )
    });
    // A few more frames: the text must not be copied again.
    for _ in 0..5 {
        f.harness.step();
        collect(&f.harness);
    }
    settle(&mut f.harness);
    copied
}
