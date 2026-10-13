//! The dialog and clipboard seams of the app, with a fake picker: no native dialog opens here.
use super::compare::tests::export::plan_modal;
use super::file_picker::{FilePicker, PickPoll, start_pick, suggested_file};
use super::*;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::{
    ClipboardText, ExportDraft, ExportKind, ExportUi, PlanOptions, ReportOptions,
};
use curlybrace_core::compare::protocol::CompareCommand;
use curlybrace_core::compare::protocol::ExportFormat;
use curlybrace_core::secrets::MemorySecretStore;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Instant;

/// Answers with a canned path (or `None`), counts the calls and, when gated, blocks each call
/// until the test releases it.
struct FakePicker {
    answer: Option<PathBuf>,
    calls: AtomicUsize,
    asked: Mutex<Vec<(String, String)>>,
    gate: Option<Mutex<Receiver<()>>>,
}

impl FakePicker {
    fn new(answer: Option<&str>) -> Arc<Self> {
        Arc::new(Self::build(answer, None))
    }
    /// A picker whose dialog stays open until the returned sender fires.
    fn gated(answer: Option<&str>) -> (Arc<Self>, Sender<()>) {
        let (release, gate) = channel();
        (
            Arc::new(Self::build(answer, Some(Mutex::new(gate)))),
            release,
        )
    }
    fn build(answer: Option<&str>, gate: Option<Mutex<Receiver<()>>>) -> Self {
        Self {
            answer: answer.map(PathBuf::from),
            calls: AtomicUsize::new(0),
            asked: Mutex::new(Vec::new()),
            gate,
        }
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
        if let Some(gate) = &self.gate {
            let _ = gate.lock().unwrap().recv();
        }
        self.answer.clone()
    }
}

const WAIT_SECONDS: u64 = 5;

/// Blocks until the picker has been called (the worker is a real thread).
fn wait_for_calls(picker: &FakePicker, expected: usize) {
    let start = Instant::now();
    while picker.calls() < expected {
        assert!(
            start.elapsed().as_secs() < WAIT_SECONDS,
            "picker not called"
        );
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

type Commands = tokio::sync::mpsc::UnboundedReceiver<curlybrace_core::runtime::Command>;

fn bare_app() -> (CurlybraceApp, Commands) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    // The event sender lives on with the app so the receiver never reports a closed runtime.
    std::mem::forget(_events);
    let secrets = Arc::new(MemorySecretStore::default());
    let app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        Vec::new(),
        secrets,
    );
    (app, commands)
}

fn app_with(picker: Arc<dyn FilePicker>) -> (CurlybraceApp, Commands) {
    let (app, commands) = bare_app();
    (app.with_file_picker(picker, Arc::new(|| {})), commands)
}

/// A frame: pump, draw, apply, then the self-healing dialog start and the clipboard.
fn frame(app: &mut CurlybraceApp, ctx: &egui::Context) -> Vec<String> {
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

/// Runs frames until the dialog in flight has been answered and consumed.
fn frames_until_answered(app: &mut CurlybraceApp, ctx: &egui::Context) {
    let start = Instant::now();
    loop {
        frame(app, ctx);
        if app.pick.is_none() {
            return;
        }
        assert!(
            start.elapsed().as_secs() < WAIT_SECONDS,
            "dialog never answered"
        );
        std::thread::yield_now();
    }
}

fn choosing_app(picker: Arc<dyn FilePicker>) -> (CurlybraceApp, Commands) {
    let (mut app, commands) = app_with(picker);
    *app.state_mut() = plan_modal();
    app.dispatch(Input::Compare(CompareAction::ChooseExportFile));
    assert!(matches!(app.state().compare.export, ExportUi::Choosing(_)));
    (app, commands)
}

fn sent(commands: &mut Commands) -> Vec<curlybrace_core::runtime::Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

#[test]
fn a_chosen_path_starts_the_export_with_that_path_exactly_once() {
    let picker = FakePicker::new(Some("/out/plan.js"));
    let (mut app, mut commands) = choosing_app(picker.clone());
    let ctx = egui::Context::default();
    frames_until_answered(&mut app, &ctx);
    frame(&mut app, &ctx);
    assert_eq!(picker.calls(), 1);
    let commands = sent(&mut commands);
    assert_eq!(commands.len(), 1, "one command: {commands:?}");
    assert!(matches!(
        &commands[0],
        curlybrace_core::runtime::Command::Compare(CompareCommand::Export { path, .. })
            if path == &PathBuf::from("/out/plan.js")
    ));
    assert!(app.state().compare.export.running_run().is_some());
}

#[test]
fn a_cancelled_dialog_returns_to_configuring_and_does_not_reopen() {
    let picker = FakePicker::new(None);
    let (mut app, mut commands) = choosing_app(picker.clone());
    let ctx = egui::Context::default();
    frames_until_answered(&mut app, &ctx);
    assert!(matches!(
        app.state().compare.export,
        ExportUi::Configuring(_)
    ));
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    assert_eq!(picker.calls(), 1, "no second dialog by itself");
    assert!(app.pick.is_none());
    assert!(sent(&mut commands).is_empty());
}

#[test]
fn choosing_opens_exactly_one_dialog_with_the_suggested_name() {
    let (picker, release) = FakePicker::gated(None);
    let (mut app, _commands) = choosing_app(picker.clone());
    let ctx = egui::Context::default();
    // Several frames while the dialog is still open: still one dialog (deterministic: the
    // worker is blocked on the gate until released).
    for _ in 0..4 {
        frame(&mut app, &ctx);
    }
    wait_for_calls(&picker, 1);
    for _ in 0..4 {
        frame(&mut app, &ctx);
    }
    assert_eq!(picker.calls(), 1);
    assert_eq!(
        picker.asked.lock().unwrap().as_slice(),
        [("curlybrace-plan.js".to_owned(), "js".to_owned())]
    );
    release.send(()).unwrap();
    frames_until_answered(&mut app, &ctx);
    assert_eq!(picker.calls(), 1);
}

#[test]
fn nothing_opens_unless_an_export_waits_in_choosing() {
    let picker = FakePicker::new(None);
    let (mut app, _commands) = app_with(picker.clone());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    assert_eq!(picker.calls(), 0, "nothing waiting, nothing opens");
    *app.state_mut() = plan_modal();
    frame(&mut app, &ctx);
    assert_eq!(picker.calls(), 0, "Configuring opens nothing");
}

#[test]
fn a_late_answer_for_a_closed_or_changed_export_is_dropped() {
    for stale in [
        ExportUi::Idle,
        ExportUi::Configuring(draft()),
        ExportUi::Choosing(ExportDraft {
            kind: ExportKind::Report,
            ..draft()
        }),
    ] {
        let (picker, release) = FakePicker::gated(Some("/out/plan.js"));
        let (mut app, mut commands) = choosing_app(picker.clone());
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        wait_for_calls(&picker, 1);
        // The export moves on while the dialog is open (closed / back to the modal / new draft).
        app.state_mut().compare.export = stale.clone();
        release.send(()).unwrap();
        let start = Instant::now();
        while app
            .pick
            .as_ref()
            .is_some_and(|p| p.poll() == PickPoll::Waiting)
        {
            assert!(start.elapsed().as_secs() < WAIT_SECONDS, "never answered");
            std::thread::yield_now();
        }
        app.pump();
        assert!(
            sent(&mut commands).is_empty(),
            "stale answer dispatched: {stale:?}"
        );
        assert_eq!(app.state().compare.export, stale, "state untouched");
    }
}

#[test]
fn a_late_answer_after_the_person_cancelled_while_waiting_starts_nothing() {
    let (picker, release) = FakePicker::gated(Some("/out/plan.js"));
    let (mut app, mut commands) = choosing_app(picker.clone());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    wait_for_calls(&picker, 1);
    // Cancel in the modal while the dialog is still open.
    app.dispatch(Input::Compare(CompareAction::ExportPathCancelled));
    assert!(matches!(
        app.state().compare.export,
        ExportUi::Configuring(_)
    ));
    release.send(()).unwrap();
    let start = Instant::now();
    while app
        .pick
        .as_ref()
        .is_some_and(|p| p.poll() == PickPoll::Waiting)
    {
        assert!(start.elapsed().as_secs() < WAIT_SECONDS, "never answered");
        std::thread::yield_now();
    }
    for _ in 0..3 {
        frame(&mut app, &ctx);
    }
    assert!(
        sent(&mut commands).is_empty(),
        "the late answer started a job"
    );
    assert!(matches!(
        app.state().compare.export,
        ExportUi::Configuring(_)
    ));
}

#[test]
fn a_new_draft_after_a_stale_answer_gets_its_own_dialog() {
    let (picker, release) = FakePicker::gated(None);
    let (mut app, _commands) = choosing_app(picker.clone());
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    wait_for_calls(&picker, 1);
    let report = ExportDraft {
        kind: ExportKind::Report,
        ..draft()
    };
    app.state_mut().compare.export = ExportUi::Choosing(report);
    release.send(()).unwrap();
    // The stale answer is dropped, then the current draft opens a dialog of its own.
    let start = Instant::now();
    while picker.calls() < 2 {
        frame(&mut app, &ctx);
        assert!(start.elapsed().as_secs() < WAIT_SECONDS, "no new dialog");
        std::thread::yield_now();
    }
    release.send(()).unwrap();
    assert_eq!(
        picker.asked.lock().unwrap()[1],
        ("curlybrace-report.csv".to_owned(), "csv".to_owned())
    );
}

#[test]
fn the_default_picker_cancels_without_opening_anything() {
    let (mut app, _commands) = bare_app();
    *app.state_mut() = plan_modal();
    app.dispatch(Input::Compare(CompareAction::ChooseExportFile));
    let ctx = egui::Context::default();
    frames_until_answered(&mut app, &ctx);
    assert!(matches!(
        app.state().compare.export,
        ExportUi::Configuring(_)
    ));
    assert_eq!(file_picker::NoFilePicker.save_file("a", "js"), None);
}

#[test]
fn the_reducer_never_asks_a_picker() {
    let picker = FakePicker::new(Some("/out/plan.js"));
    let (mut app, _commands) = app_with(picker.clone());
    *app.state_mut() = plan_modal();
    app.dispatch(Input::Compare(CompareAction::ChooseExportFile));
    app.dispatch(Input::Compare(CompareAction::ExportPathCancelled));
    assert_eq!(picker.calls(), 0, "only a frame opens dialogs");
}

#[test]
fn a_dead_worker_reads_as_cancelled() {
    struct Panics;
    impl FilePicker for Panics {
        fn save_file(&self, _: &str, _: &str) -> Option<PathBuf> {
            panic!("dialog crashed")
        }
    }
    let pending = start_pick(Arc::new(Panics), draft(), "a", "js", Arc::new(|| {}));
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
    let pending = start_pick(FakePicker::new(None), draft(), "a", "js", repaint);
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

#[test]
fn the_clipboard_text_is_copied_exactly_once_and_leaves_the_state() {
    let (mut app, _commands) = bare_app();
    app.state_mut().compare.export = ExportUi::Copied {
        rows: 2,
        text: Some(ClipboardText::new("pair,key\r\n1,k\r\n".into())),
    };
    let ctx = egui::Context::default();
    assert_eq!(frame(&mut app, &ctx), ["pair,key\r\n1,k\r\n"]);
    assert!(matches!(
        app.state().compare.export,
        ExportUi::Copied {
            rows: 2,
            text: None
        }
    ));
    // The next frames copy nothing more.
    assert!(frame(&mut app, &ctx).is_empty());
}

#[test]
fn no_clipboard_text_copies_nothing() {
    let (mut app, _commands) = bare_app();
    let ctx = egui::Context::default();
    assert!(frame(&mut app, &ctx).is_empty());
}
