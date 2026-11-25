//! The document export in the app: the modal, the file dialog, the job and its events. Pure
//! reducer tests, plus the dialog seam with a fake picker (no native dialog opens here).
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::export::protocol::{
    DocExportOutcome, DocFormat, ExportCommand, ExportEvent, JsonMode,
};
use curlybrace_core::pipeline_options::CollationOption;
use curlybrace_core::query_options::QueryOptions;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Query};
use eframe::egui;
use uuid::Uuid;

use super::doc_export::DocExportAction;
use super::file_picker::{FilePicker, suggested_doc_file};
use super::reducer::{Input, Out, update};
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::doc_export::{DocExportDraft, DocExportUi};

fn sid() -> Uuid {
    Uuid::from_u128(1)
}

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn collated() -> Query {
    Query {
        filter: doc! { "a": 1 },
        options: QueryOptions {
            collation: Some(CollationOption {
                locale: "en".into(),
                strength: None,
                case_level: None,
                case_first: None,
                numeric_ordering: None,
                alternate: None,
            }),
            ..QueryOptions::default()
        },
        ..Query::default()
    }
}

fn connected() -> AppState {
    AppState {
        open: vec![sid()],
        active: Some(sid()),
        status: ConnStatus::Connected,
        page_query: Query {
            filter: doc! { "status": "open" },
            ..Query::default()
        },
        ..AppState::default()
    }
}

fn act(state: AppState, action: DocExportAction) -> Out {
    update(state, Input::DocExport(action))
}

fn open(state: AppState, whole: bool) -> AppState {
    act(
        state,
        DocExportAction::Open {
            ns: ns(),
            whole_collection: whole,
        },
    )
    .0
}

fn choosing(state: AppState) -> AppState {
    act(open(state, false), DocExportAction::ChooseFile).0
}

fn running(state: AppState, path: &str) -> (AppState, Vec<Command>) {
    act(
        choosing(state),
        DocExportAction::PathChosen(PathBuf::from(path)),
    )
}

fn run_of(state: &AppState) -> u64 {
    match &state.doc_export {
        DocExportUi::Running { run, .. } => *run,
        other => panic!("not running: {other:?}"),
    }
}

fn event(e: ExportEvent) -> Input {
    Input::Event(Event::Export(e))
}

#[test]
fn open_carries_the_applied_filter() {
    let state = open(connected(), false);
    match &state.doc_export {
        DocExportUi::Configuring { source, draft } => {
            assert_eq!(source.filter, Some(doc! { "status": "open" }));
            assert_eq!(source.ns, ns());
            assert_eq!(source.session, sid());
            assert_eq!(*draft, DocExportDraft::default());
        }
        other => panic!("not configuring: {other:?}"),
    }
}

#[test]
fn open_for_the_whole_collection_has_no_filter_and_ignores_a_page_collation() {
    let mut state = connected();
    state.page_query = collated();
    let state = open(state, true);
    match &state.doc_export {
        DocExportUi::Configuring { source, .. } => assert_eq!(source.filter, None),
        other => panic!("not configuring: {other:?}"),
    }
}

#[test]
fn open_is_refused_for_a_collated_query() {
    let mut state = connected();
    state.page_query = collated();
    let (state, commands) = act(
        state,
        DocExportAction::Open {
            ns: ns(),
            whole_collection: false,
        },
    );
    assert_eq!(state.doc_export, DocExportUi::Idle);
    assert!(commands.is_empty());
}

#[test]
fn open_is_refused_when_not_connected() {
    let state = AppState {
        status: ConnStatus::Disconnected,
        ..connected()
    };
    assert_eq!(open(state, true).doc_export, DocExportUi::Idle);
}

#[test]
fn edits_change_the_draft() {
    let state = open(connected(), false);
    let draft = DocExportDraft {
        format: DocFormat::JsonArray,
        mode: JsonMode::Relaxed,
    };
    let state = act(state, DocExportAction::Edit(draft)).0;
    assert!(matches!(
        state.doc_export,
        DocExportUi::Configuring { draft: d, .. } if d == draft
    ));
}

#[test]
fn choose_file_moves_to_choosing_and_sends_nothing() {
    let (state, commands) = act(open(connected(), false), DocExportAction::ChooseFile);
    assert!(matches!(state.doc_export, DocExportUi::Choosing { .. }));
    assert!(commands.is_empty());
}

#[test]
fn a_chosen_path_sends_exactly_one_start() {
    let draft = DocExportDraft {
        format: DocFormat::JsonArray,
        mode: JsonMode::Relaxed,
    };
    let state = act(open(connected(), false), DocExportAction::Edit(draft)).0;
    let state = act(state, DocExportAction::ChooseFile).0;
    let (state, commands) = act(
        state,
        DocExportAction::PathChosen(PathBuf::from("/out/orders.json")),
    );
    assert_eq!(commands.len(), 1);
    let Command::Export(ExportCommand::Start { run, request }) = &commands[0] else {
        panic!("not a start: {:?}", commands[0]);
    };
    assert_eq!(request.session, sid());
    assert_eq!(request.ns, ns());
    assert_eq!(request.filter, doc! { "status": "open" });
    assert_eq!(request.format, DocFormat::JsonArray);
    assert_eq!(request.mode, JsonMode::Relaxed);
    assert_eq!(request.path, PathBuf::from("/out/orders.json"));
    assert_eq!(run_of(&state), *run);
}

#[test]
fn a_whole_collection_start_has_an_empty_filter() {
    let state = act(open(connected(), true), DocExportAction::ChooseFile).0;
    let (_, commands) = act(
        state,
        DocExportAction::PathChosen(PathBuf::from("/o.ndjson")),
    );
    let Command::Export(ExportCommand::Start { request, .. }) = &commands[0] else {
        panic!("not a start");
    };
    assert!(request.filter.is_empty());
}

#[test]
fn a_path_without_an_open_dialog_sends_nothing() {
    let (state, commands) = act(
        open(connected(), false),
        DocExportAction::PathChosen(PathBuf::from("/o.ndjson")),
    );
    assert!(commands.is_empty());
    assert!(matches!(state.doc_export, DocExportUi::Configuring { .. }));
}

#[test]
fn a_start_after_the_connection_dropped_sends_nothing_and_reopens_the_modal() {
    let mut state = choosing(connected());
    state.status = ConnStatus::Disconnected;
    let (state, commands) = act(
        state,
        DocExportAction::PathChosen(PathBuf::from("/o.ndjson")),
    );
    assert!(commands.is_empty());
    assert!(matches!(state.doc_export, DocExportUi::Configuring { .. }));
}

#[test]
fn a_cancelled_dialog_returns_to_configuring() {
    let (state, commands) = act(choosing(connected()), DocExportAction::PathCancelled);
    assert!(matches!(state.doc_export, DocExportUi::Configuring { .. }));
    assert!(commands.is_empty());
}

#[test]
fn progress_and_done_update_the_state_and_ignore_other_runs() {
    let (state, _) = running(connected(), "/o.ndjson");
    let run = run_of(&state);
    let (state, _) = update(
        state,
        event(ExportEvent::Progress {
            run,
            done: 3,
            total: Some(10),
        }),
    );
    assert!(matches!(
        state.doc_export,
        DocExportUi::Running {
            done: 3,
            total: Some(10),
            ..
        }
    ));
    let (state, _) = update(
        state,
        event(ExportEvent::Progress {
            run: run + 1,
            done: 9,
            total: Some(10),
        }),
    );
    assert!(matches!(
        state.doc_export,
        DocExportUi::Running { done: 3, .. }
    ));
    let outcome = DocExportOutcome {
        written: 10,
        ..DocExportOutcome::default()
    };
    let (state, _) = update(state, event(ExportEvent::Done { run, outcome }));
    assert!(matches!(
        state.doc_export,
        DocExportUi::Finished { outcome: Ok(o), .. } if o.written == 10
    ));
}

#[test]
fn cancel_sends_one_cancel() {
    let (state, _) = running(connected(), "/o.ndjson");
    let run = run_of(&state);
    let (state, commands) = act(state, DocExportAction::Cancel);
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        &commands[0],
        Command::Export(ExportCommand::Cancel { run: r }) if *r == run
    ));
    let (_, again) = act(state, DocExportAction::Cancel);
    assert!(again.is_empty());
}

#[test]
fn failed_reopens_the_modal_with_a_banner() {
    let (state, _) = running(connected(), "/o.ndjson");
    let run = run_of(&state);
    let error = CoreError::Unsupported("boom".into());
    let (state, _) = update(state, event(ExportEvent::Failed { run, error }));
    assert!(matches!(state.doc_export, DocExportUi::Configuring { .. }));
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "export_documents",
            ..
        })
    ));
}

#[test]
fn a_stale_failure_changes_nothing() {
    let (state, _) = running(connected(), "/o.ndjson");
    let run = run_of(&state);
    let error = CoreError::Unsupported("boom".into());
    let (state, _) = update(
        state,
        event(ExportEvent::Failed {
            run: run + 1,
            error,
        }),
    );
    assert!(matches!(state.doc_export, DocExportUi::Running { .. }));
    assert!(state.banner.is_none());
}

#[test]
fn a_second_open_while_running_is_ignored() {
    let (state, _) = running(connected(), "/o.ndjson");
    let (state, commands) = act(
        state,
        DocExportAction::Open {
            ns: Namespace::new("other", "coll"),
            whole_collection: true,
        },
    );
    assert!(commands.is_empty());
    assert!(matches!(&state.doc_export, DocExportUi::Running { source, .. } if source.ns == ns()));
}

#[test]
fn close_keeps_a_running_export_and_ends_a_finished_one() {
    let (state, _) = running(connected(), "/o.ndjson");
    let run = run_of(&state);
    let state = act(state, DocExportAction::Close).0;
    assert!(matches!(state.doc_export, DocExportUi::Running { .. }));
    let (state, _) = update(
        state,
        event(ExportEvent::Done {
            run,
            outcome: DocExportOutcome::default(),
        }),
    );
    assert_eq!(
        act(state, DocExportAction::Close).0.doc_export,
        DocExportUi::Idle
    );
}

#[test]
fn the_suggested_file_follows_the_format() {
    assert_eq!(suggested_doc_file(DocFormat::Ndjson), ("export", "ndjson"));
    assert_eq!(suggested_doc_file(DocFormat::JsonArray), ("export", "json"));
}

// The dialog seam: a fake picker records what the app asked for.

struct FakePicker {
    answer: Option<PathBuf>,
    calls: AtomicUsize,
    asked: Mutex<Vec<(String, String)>>,
}

impl FilePicker for FakePicker {
    fn save_file(&self, name: &str, extension: &str) -> Option<PathBuf> {
        self.asked
            .lock()
            .unwrap()
            .push((name.to_owned(), extension.to_owned()));
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }
}

type Commands = tokio::sync::mpsc::UnboundedReceiver<Command>;

fn app_with(picker: Arc<dyn FilePicker>) -> (super::CurlybraceApp, Commands) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, rx) = tokio::sync::mpsc::unbounded_channel();
    std::mem::forget(events);
    let secrets = Arc::new(curlybrace_core::secrets::MemorySecretStore::default());
    let app = super::CurlybraceApp::new(
        super::RuntimeHandle { tx, rx },
        crate::i18n::I18n::english(),
        Vec::new(),
        secrets,
    );
    (app.with_file_picker(picker, Arc::new(|| {})), commands)
}

fn frames_until_answered(app: &mut super::CurlybraceApp) {
    let ctx = egui::Context::default();
    let start = Instant::now();
    loop {
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
        if app.doc_pick.is_none() {
            return;
        }
        assert!(start.elapsed().as_secs() < 5, "dialog never answered");
        std::thread::yield_now();
    }
}

fn dialog_round(
    format: DocFormat,
    answer: Option<&str>,
) -> (Arc<FakePicker>, AppState, Vec<Command>) {
    let picker = Arc::new(FakePicker {
        answer: answer.map(PathBuf::from),
        calls: AtomicUsize::new(0),
        asked: Mutex::new(Vec::new()),
    });
    let (mut app, mut commands) = app_with(picker.clone());
    *app.state_mut() = connected();
    app.dispatch(Input::DocExport(DocExportAction::Open {
        ns: ns(),
        whole_collection: false,
    }));
    app.dispatch(Input::DocExport(DocExportAction::Edit(DocExportDraft {
        format,
        ..DocExportDraft::default()
    })));
    app.dispatch(Input::DocExport(DocExportAction::ChooseFile));
    frames_until_answered(&mut app);
    let sent: Vec<Command> = std::iter::from_fn(|| commands.try_recv().ok()).collect();
    let state = std::mem::take(app.state_mut());
    (picker, state, sent)
}

#[test]
fn the_dialog_asks_for_the_ndjson_suffix_and_a_path_starts_the_export() {
    let (picker, state, sent) = dialog_round(DocFormat::Ndjson, Some("/out/orders.ndjson"));
    assert_eq!(
        picker.asked.lock().unwrap().as_slice(),
        &[("export".to_owned(), "ndjson".to_owned())]
    );
    assert!(matches!(state.doc_export, DocExportUi::Running { .. }));
    let starts = sent
        .iter()
        .filter(|c| matches!(c, Command::Export(ExportCommand::Start { .. })))
        .count();
    assert_eq!(starts, 1);
}

#[test]
fn the_dialog_asks_for_the_json_suffix_for_an_array() {
    let (picker, state, sent) = dialog_round(DocFormat::JsonArray, None);
    assert_eq!(
        picker.asked.lock().unwrap().as_slice(),
        &[("export".to_owned(), "json".to_owned())]
    );
    assert!(matches!(state.doc_export, DocExportUi::Configuring { .. }));
    assert!(sent.is_empty());
}

#[test]
fn debug_hides_the_path_and_the_namespace() {
    let chosen = format!(
        "{:?}",
        DocExportAction::PathChosen(PathBuf::from("/home/someone/secret.ndjson"))
    );
    assert!(!chosen.contains("secret"), "{chosen}");
    let open = format!(
        "{:?}",
        DocExportAction::Open {
            ns: Namespace::new("secretdb", "secretcoll"),
            whole_collection: false,
        }
    );
    assert!(!open.contains("secret"), "{open}");
}

#[test]
fn choose_file_is_refused_after_another_session_became_the_shown_one() {
    let state = open(connected(), false);
    let other = AppState {
        active: Some(Uuid::from_u128(2)),
        ..state
    };
    let (state, commands) = act(other, DocExportAction::ChooseFile);
    assert!(matches!(state.doc_export, DocExportUi::Configuring { .. }));
    assert!(commands.is_empty());
}

/// The first dialog stays open until `release` is sent; later dialogs answer at once.
struct GatedPicker {
    calls: AtomicUsize,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl FilePicker for GatedPicker {
    fn save_file(&self, _name: &str, _extension: &str) -> Option<PathBuf> {
        match self.calls.fetch_add(1, Ordering::SeqCst) {
            0 => {
                let _ = self.release.lock().unwrap().recv();
                Some(PathBuf::from("/out/first.ndjson"))
            }
            _ => Some(PathBuf::from("/out/second.json")),
        }
    }
}

fn run_frame_once(app: &mut super::CurlybraceApp) {
    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
}

#[test]
fn going_back_while_the_dialog_is_open_lets_a_new_choose_file_open_a_fresh_one() {
    let (release, gate) = std::sync::mpsc::channel();
    let picker = Arc::new(GatedPicker {
        calls: AtomicUsize::new(0),
        release: Mutex::new(gate),
    });
    let (mut app, mut commands) = app_with(picker.clone());
    *app.state_mut() = connected();
    let act = |app: &mut super::CurlybraceApp, a| app.dispatch(Input::DocExport(a));
    act(
        &mut app,
        DocExportAction::Open {
            ns: ns(),
            whole_collection: false,
        },
    );
    act(&mut app, DocExportAction::ChooseFile);
    run_frame_once(&mut app);
    // Back while the first dialog is still open, then another format and a new Choose file.
    act(&mut app, DocExportAction::PathCancelled);
    act(
        &mut app,
        DocExportAction::Edit(DocExportDraft {
            format: DocFormat::JsonArray,
            ..DocExportDraft::default()
        }),
    );
    act(&mut app, DocExportAction::ChooseFile);
    frames_until_answered(&mut app);
    assert_eq!(picker.calls.load(Ordering::SeqCst), 2);
    let _ = release.send(());
    let state = std::mem::take(app.state_mut());
    assert!(matches!(state.doc_export, DocExportUi::Running { .. }));
    let started = std::iter::from_fn(|| commands.try_recv().ok()).find_map(|c| match c {
        Command::Export(ExportCommand::Start { request, .. }) => Some(request.path),
        _ => None,
    });
    assert_eq!(started, Some(PathBuf::from("/out/second.json")));
}

fn ignores_options(state: &AppState) -> bool {
    match &state.doc_export {
        DocExportUi::Configuring { source, .. } => source.ignores_options,
        other => panic!("not configuring: {other:?}"),
    }
}

#[test]
fn open_notes_a_sort_or_projection_the_export_will_not_apply() {
    assert!(!ignores_options(&open(connected(), false)));
    for empty in [
        (|q: &mut Query| q.sort = Some(doc! {})) as fn(&mut Query),
        |q: &mut Query| q.projection = Some(doc! {}),
    ] {
        let mut state = connected();
        empty(&mut state.page_query);
        assert!(!ignores_options(&open(state, false)));
    }
    for adjust in [
        (|q: &mut Query| q.sort = Some(doc! { "a": 1 })) as fn(&mut Query),
        |q: &mut Query| q.projection = Some(doc! { "a": 1 }),
    ] {
        let shaped = || {
            let mut state = connected();
            adjust(&mut state.page_query);
            state
        };
        assert!(ignores_options(&open(shaped(), false)));
        // A whole collection export reads no query, so there is nothing to note.
        assert!(!ignores_options(&open(shaped(), true)));
    }
}
