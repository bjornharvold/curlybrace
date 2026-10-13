//! The dialog and clipboard seams of the app, with a fake picker: no native dialog opens here.
use super::file_picker::{FilePicker, PickPoll, start_pick, suggested_file};
use super::*;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{
    ClipboardText, ExportDraft, ExportKind, ExportUi, PlanOptions, ReportOptions,
};
use curlybrace_core::compare::protocol::ExportFormat;
use curlybrace_core::secrets::MemorySecretStore;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Answers with a canned path (or `None`) and counts how often it was asked.
struct FakePicker {
    answer: Option<PathBuf>,
    calls: AtomicUsize,
    asked: Mutex<Vec<(String, String)>>,
}

impl FakePicker {
    fn new(answer: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            answer: answer.map(PathBuf::from),
            calls: AtomicUsize::new(0),
            asked: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl FilePicker for FakePicker {
    fn save_file(&self, name: &str, extension: &str) -> Option<PathBuf> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.asked
            .lock()
            .unwrap()
            .push((name.to_owned(), extension.to_owned()));
        self.answer.clone()
    }
}

/// Blocks until the picker has been called (or fails): the worker is a real thread.
fn wait_for_calls(picker: &FakePicker, expected: usize) {
    let start = Instant::now();
    while picker.calls() < expected {
        assert!(start.elapsed().as_secs() < 5, "picker was never called");
        std::thread::yield_now();
    }
}

fn draft() -> ExportDraft {
    ExportDraft {
        kind: ExportKind::Plan,
        plan: PlanOptions::default(),
        report: ReportOptions::default(),
    }
}

fn app_with(picker: Arc<dyn FilePicker>) -> CurlybraceApp {
    let (tx, _commands) = tokio::sync::mpsc::unbounded_channel();
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    // Keep the command receiver alive for the whole test: sending must not mark the runtime dead.
    std::mem::forget(_commands);
    let handle = RuntimeHandle { tx, rx };
    let secrets = Arc::new(MemorySecretStore::default());
    let mut app = CurlybraceApp::new(handle, I18n::english(), Vec::new(), secrets)
        .with_file_picker(picker, Arc::new(|| {}));
    app.state_mut().compare.export = ExportUi::Choosing(draft());
    app
}

fn pick_event() -> UiEvent {
    UiEvent::PickExportFile {
        suggested_name: "curlybrace-plan.js",
        extension: "js",
    }
}

/// Pumps until the pending dialog has been answered (the reducer then leaves `Choosing`).
fn pump_until_answered(app: &mut CurlybraceApp) {
    let start = Instant::now();
    while app.pick.is_some() {
        assert!(start.elapsed().as_secs() < 5, "dialog never answered");
        app.pump();
        std::thread::yield_now();
    }
}

#[test]
fn a_chosen_path_is_dispatched_as_export_path_chosen_exactly_once() {
    let picker = FakePicker::new(Some("/out/plan.js"));
    let mut app = app_with(picker.clone());
    app.apply_ui_event(pick_event());
    pump_until_answered(&mut app);
    assert_eq!(picker.calls(), 1);
    assert_eq!(
        picker.asked.lock().unwrap().as_slice(),
        [("curlybrace-plan.js".to_owned(), "js".to_owned())]
    );
    // The reducer moved on (Running, or back to Configuring when the runtime refused): the
    // answer was consumed; pumping again changes nothing and asks nobody.
    let after = format!("{:?}", app.state().compare.export);
    assert!(!matches!(app.state().compare.export, ExportUi::Choosing(_)));
    app.pump();
    assert_eq!(format!("{:?}", app.state().compare.export), after);
    assert_eq!(picker.calls(), 1);
}

#[test]
fn a_cancelled_dialog_returns_to_configuring() {
    let picker = FakePicker::new(None);
    let mut app = app_with(picker.clone());
    app.apply_ui_event(pick_event());
    pump_until_answered(&mut app);
    assert!(matches!(
        app.state().compare.export,
        ExportUi::Configuring(_)
    ));
}

#[test]
fn a_second_request_while_a_dialog_is_pending_is_ignored() {
    let picker = FakePicker::new(None);
    let mut app = app_with(picker.clone());
    app.apply_ui_event(pick_event());
    app.apply_ui_event(pick_event());
    wait_for_calls(&picker, 1);
    pump_until_answered(&mut app);
    assert_eq!(picker.calls(), 1, "one dialog for two requests");
}

#[test]
fn a_request_outside_choosing_never_opens_a_dialog() {
    let picker = FakePicker::new(Some("/out/plan.js"));
    let mut app = app_with(picker.clone());
    app.state_mut().compare.export = ExportUi::Configuring(draft());
    app.apply_ui_event(pick_event());
    app.pump();
    assert!(app.pick.is_none());
    assert_eq!(picker.calls(), 0);
}

#[test]
fn the_default_picker_cancels_and_a_dispatch_never_asks_a_picker() {
    let picker = FakePicker::new(Some("/out/plan.js"));
    let mut app = app_with(picker.clone());
    // Everything the reducer does with the dialog actions leaves the picker alone.
    app.dispatch(Input::Compare(CompareAction::ChooseExportFile));
    app.dispatch(Input::Compare(CompareAction::ExportPathCancelled));
    assert_eq!(picker.calls(), 0);
    assert_eq!(file_picker::NoFilePicker.save_file("a", "js"), None);
}

#[test]
fn a_dead_worker_reads_as_cancelled() {
    struct Panics;
    impl FilePicker for Panics {
        fn save_file(&self, _: &str, _: &str) -> Option<PathBuf> {
            panic!("dialog crashed")
        }
    }
    let pending = start_pick(Arc::new(Panics), "a", "js", Arc::new(|| {}));
    let start = Instant::now();
    loop {
        match pending.poll() {
            PickPoll::Done(path) => {
                assert_eq!(path, None);
                break;
            }
            PickPoll::Waiting => {
                assert!(start.elapsed().as_secs() < 5, "never resolved");
                std::thread::yield_now();
            }
        }
    }
}

#[test]
fn the_worker_wakes_the_ui_after_answering() {
    let woken = Arc::new(AtomicUsize::new(0));
    let repaint: Repaint = {
        let woken = woken.clone();
        Arc::new(move || {
            woken.fetch_add(1, Ordering::SeqCst);
        })
    };
    let pending = start_pick(FakePicker::new(None), "a", "js", repaint);
    let start = Instant::now();
    while woken.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed().as_secs() < 5, "never woken");
        std::thread::yield_now();
    }
    assert_eq!(pending.poll(), PickPoll::Done(None));
}

#[test]
fn suggested_names_and_extensions_per_kind_and_format() {
    let table = [
        (
            ExportKind::Plan,
            ExportFormat::Mongosh,
            ("curlybrace-plan.js", "js"),
        ),
        (
            ExportKind::Plan,
            ExportFormat::JsonPlan,
            ("curlybrace-plan.json", "json"),
        ),
        (
            ExportKind::Report,
            ExportFormat::Mongosh,
            ("curlybrace-report.csv", "csv"),
        ),
        (
            ExportKind::Report,
            ExportFormat::JsonPlan,
            ("curlybrace-report.csv", "csv"),
        ),
    ];
    for (kind, format, expected) in table {
        assert_eq!(
            suggested_file(kind, format),
            expected,
            "{kind:?}/{format:?}"
        );
    }
}

/// Runs one frame and returns the texts egui was asked to put on the clipboard.
fn frame_copies(app: &mut CurlybraceApp, ctx: &egui::Context) -> Vec<String> {
    let output = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
    output
        .platform_output
        .commands
        .into_iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn the_clipboard_text_is_copied_exactly_once_and_leaves_the_state() {
    let mut app = app_with(FakePicker::new(None));
    app.state_mut().compare.export = ExportUi::Copied {
        rows: 2,
        text: Some(ClipboardText::new("pair,key\r\n1,k\r\n".into())),
    };
    let ctx = egui::Context::default();
    assert_eq!(frame_copies(&mut app, &ctx), ["pair,key\r\n1,k\r\n"]);
    assert!(matches!(
        app.state().compare.export,
        ExportUi::Copied {
            rows: 2,
            text: None
        }
    ));
    // The next frames copy nothing more.
    assert!(frame_copies(&mut app, &ctx).is_empty());
}

#[test]
fn no_clipboard_text_copies_nothing() {
    let mut app = app_with(FakePicker::new(None));
    let ctx = egui::Context::default();
    assert!(frame_copies(&mut app, &ctx).is_empty());
}
