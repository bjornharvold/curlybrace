//! The document export in the accesskit tree and under the keyboard: the modal's radios, notes,
//! buttons and progress bar, the finished results, the two entry points with their reasons, and
//! the whole flow in the real app with a fake file dialog and a scripted event stream. Fast,
//! offline, no GPU, and no native dialog.
mod support;

use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::export::protocol::{DocExportOutcome, DocFormat, ExportEvent, JsonMode};
use curlybrace_core::runtime::{Event, SessionId};
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::doc_export::DocExportAction;
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::viewmodel::compare::RedactedPath;
use curlybrace_ui::viewmodel::doc_export::{
    DocExportBlock, DocExportDraft, DocExportSource, DocExportUi,
};
use curlybrace_ui::views::doc_export::show_doc_export;
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::path::PathBuf;
use support::wait::{SHORT, wait_until};

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

fn ns() -> Namespace {
    Namespace::new("crm", "customers")
}

fn source(filter: bool) -> DocExportSource {
    DocExportSource {
        session: SessionId::from_u128(7),
        ns: ns(),
        filter: filter.then(|| doc! { "status": "open" }),
        ignores_options: false,
    }
}

fn configuring(filter: bool, draft: DocExportDraft) -> DocExportUi {
    DocExportUi::Configuring {
        source: source(filter),
        draft,
    }
}

fn running(total: Option<u64>, done: u64, cancel_requested: bool) -> DocExportUi {
    DocExportUi::Running {
        run: 1,
        source: source(false),
        draft: DocExportDraft::default(),
        path: RedactedPath::new(PathBuf::from("/out/customers.ndjson")),
        total,
        done,
        cancel_requested,
    }
}

fn finished(outcome: Result<DocExportOutcome, CoreError>) -> DocExportUi {
    DocExportUi::Finished {
        outcome,
        path: Some(RedactedPath::new(PathBuf::from("/out/customers.ndjson"))),
    }
}

fn outcome(written: u64, cancelled: bool, file_left: bool) -> DocExportOutcome {
    DocExportOutcome {
        written,
        bytes: 1234,
        cancelled,
        file_left,
        file_created: true,
    }
}

struct Run {
    ui: DocExportUi,
    actions: Vec<DocExportAction>,
}

fn harness(ui: DocExportUi) -> Harness<'static, Run> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 900.0))
        .build_state(
            |ctx, run: &mut Run| {
                if let Some(action) = show_doc_export(ctx, &run.ui, &I18n::english()) {
                    run.actions.push(action);
                }
            },
            Run {
                ui,
                actions: Vec::new(),
            },
        );
    harness.run_steps(5);
    harness
}

fn actions(h: &Harness<'_, Run>) -> Vec<DocExportAction> {
    h.state().actions.clone()
}

fn shown(h: &Harness<'_, Run>, text: &str) -> bool {
    h.query_all_by_label(text).count() > 0
}

// ---- the options ----

#[test]
fn the_options_name_every_control_and_note() {
    let h = harness(configuring(false, DocExportDraft::default()));
    h.get_by_label(&t("export.title"));
    h.get_by_label(&tw(
        "export.source.collection",
        &[("name", "crm.customers")],
    ));
    for key in [
        "export.format.ndjson",
        "export.format.json_array",
        "export.mode.canonical",
        "export.mode.relaxed",
    ] {
        h.get_by_role_and_label(Role::RadioButton, &t(key));
    }
    h.get_by_role_and_label(Role::Button, &t("export.choose_file"));
    h.get_by_role_and_label(Role::Button, &t("export.finished.close"));
    for key in [
        "export.format.title",
        "export.mode.title",
        "export.mode.canonical_hint",
        "export.mode.relaxed_hint",
        "export.note.sensitive",
        "export.note.consistency",
    ] {
        h.get_by_label(&t(key));
    }
    assert_eq!(t("export.mode.relaxed_hint"), "Readable, not type-exact");
}

#[test]
fn a_query_export_names_its_source_as_a_query() {
    let h = harness(configuring(true, DocExportDraft::default()));
    h.get_by_label(&tw("export.source.query", &[("name", "crm.customers")]));
}

#[test]
fn the_chosen_options_are_checked() {
    let draft = DocExportDraft {
        format: DocFormat::JsonArray,
        mode: JsonMode::Relaxed,
    };
    let h = harness(configuring(false, draft));
    let checked = |key: &str| {
        h.get_by_role_and_label(Role::RadioButton, &t(key))
            .accesskit_node()
            .toggled()
            == Some(egui::accesskit::Toggled::True)
    };
    assert!(checked("export.format.json_array"));
    assert!(checked("export.mode.relaxed"));
    assert!(!checked("export.format.ndjson"));
    assert!(!checked("export.mode.canonical"));
}

#[test]
fn picking_a_radio_edits_the_draft() {
    let mut h = harness(configuring(false, DocExportDraft::default()));
    h.get_by_role_and_label(Role::RadioButton, &t("export.format.json_array"))
        .click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::RadioButton, &t("export.mode.relaxed"))
        .click();
    h.run_steps(3);
    assert_eq!(
        actions(&h),
        vec![
            DocExportAction::Edit(DocExportDraft {
                format: DocFormat::JsonArray,
                mode: JsonMode::default(),
            }),
            DocExportAction::Edit(DocExportDraft {
                format: DocFormat::default(),
                mode: JsonMode::Relaxed,
            }),
        ]
    );
}

#[test]
fn choose_file_and_close_emit_their_actions_by_click() {
    let mut h = harness(configuring(false, DocExportDraft::default()));
    h.get_by_role_and_label(Role::Button, &t("export.choose_file"))
        .click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, &t("export.finished.close"))
        .click();
    h.run_steps(3);
    assert_eq!(
        actions(&h),
        vec![DocExportAction::ChooseFile, DocExportAction::Close]
    );
}

#[test]
fn enter_and_space_operate_the_buttons() {
    for key in [egui::Key::Enter, egui::Key::Space] {
        let mut h = harness(configuring(false, DocExportDraft::default()));
        h.get_by_role_and_label(Role::Button, &t("export.choose_file"))
            .focus();
        h.run_steps(2);
        h.key_press(key);
        h.run_steps(3);
        assert_eq!(actions(&h), vec![DocExportAction::ChooseFile], "{key:?}");
    }
}

#[test]
fn escape_closes_the_options() {
    let mut h = harness(configuring(false, DocExportDraft::default()));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(actions(&h), vec![DocExportAction::Close]);
}

#[test]
fn the_waiting_state_can_only_go_back() {
    let mut h = harness(DocExportUi::Choosing {
        source: source(false),
        draft: DocExportDraft::default(),
    });
    h.get_by_label(&t("export.choosing.waiting"));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(actions(&h).is_empty(), "Escape does not leave the dialog");
    h.get_by_role_and_label(Role::Button, &t("export.choosing.cancel"))
        .click();
    h.run_steps(3);
    assert_eq!(actions(&h), vec![DocExportAction::PathCancelled]);
}

// ---- running ----

#[test]
fn the_progress_bar_is_named_and_the_text_counts() {
    let h = harness(running(Some(200), 50, false));
    h.get_by_role_and_label(Role::ProgressIndicator, &t("export.running.bar_name"));
    h.get_by_label(&tw(
        "export.running.progress_many",
        &[("done", "50"), ("total", "200")],
    ));
}

#[test]
fn an_unknown_total_counts_only_what_is_done() {
    let h = harness(running(None, 50, false));
    h.get_by_role_and_label(Role::ProgressIndicator, &t("export.running.bar_name"));
    h.get_by_label(&tw(
        "export.running.progress_unknown_many",
        &[("done", "50")],
    ));
}

#[test]
fn a_passed_total_is_not_shown_and_the_bar_stays_within_bounds() {
    let h = harness(running(Some(10), 25, false));
    h.get_by_label(&tw(
        "export.running.progress_unknown_many",
        &[("done", "25")],
    ));
}

#[test]
fn one_document_reads_in_the_singular() {
    let h = harness(running(None, 1, false));
    h.get_by_label(&t("export.running.progress_unknown_one"));
    let h = harness(running(Some(1), 0, false));
    h.get_by_label(&tw("export.running.progress_one", &[("done", "0")]));
    let h = harness(finished(Ok(outcome(1, false, false))));
    h.get_by_label(&t("export.finished.done_one"));
    assert_eq!(t("export.finished.done_one"), "Exported 1 document.");
}

#[test]
fn a_query_export_with_a_sort_or_projection_says_they_are_not_applied() {
    let ignored = DocExportUi::Configuring {
        source: DocExportSource {
            ignores_options: true,
            ..source(true)
        },
        draft: DocExportDraft::default(),
    };
    harness(ignored).get_by_label(&t("export.source.query_ignored"));
    let plain = harness(configuring(true, DocExportDraft::default()));
    assert!(!shown(&plain, &t("export.source.query_ignored")));
}

#[test]
fn cancel_is_offered_once() {
    let mut h = harness(running(Some(10), 1, false));
    h.get_by_role_and_label(Role::Button, &t("export.running.cancel"))
        .click();
    h.run_steps(3);
    assert_eq!(actions(&h), vec![DocExportAction::Cancel]);

    let h = harness(running(Some(10), 1, true));
    let label = t("export.running.cancel");
    let cancel = h.get_by_role_and_label(Role::Button, &label);
    assert!(cancel.accesskit_node().is_disabled());
    h.get_by_label(&t("export.running.cancelling"));
}

#[test]
fn a_running_export_ignores_escape() {
    let mut h = harness(running(Some(10), 1, false));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(actions(&h).is_empty());
}

// ---- finished ----

#[test]
fn a_done_export_says_how_many_and_closes() {
    let mut h = harness(finished(Ok(outcome(42, false, false))));
    h.get_by_label(&tw("export.finished.done_many", &[("count", "42")]));
    assert!(!shown(&h, &t("export.finished.file_left")));
    h.get_by_role_and_label(Role::Button, &t("export.finished.close"))
        .click();
    h.run_steps(3);
    assert_eq!(actions(&h), vec![DocExportAction::Close]);
}

#[test]
fn a_cancel_before_any_file_was_created_does_not_claim_a_file_was_removed() {
    let untouched = DocExportOutcome {
        file_created: false,
        ..outcome(0, true, false)
    };
    let h = harness(finished(Ok(untouched)));
    h.get_by_label(&t("export.finished.cancelled_kept"));
    assert!(!shown(&h, &t("export.finished.cancelled")));
    assert!(!shown(&h, "/out/customers.ndjson"));
}

#[test]
fn a_cancelled_export_says_so() {
    let h = harness(finished(Ok(outcome(0, true, false))));
    h.get_by_label(&t("export.finished.cancelled"));
    assert!(!shown(&h, "/out/customers.ndjson"), "the file is gone");
}

#[test]
fn a_left_file_asks_the_person_to_delete_it() {
    let h = harness(finished(Ok(outcome(5, true, true))));
    h.get_by_label(&t("export.finished.cancelled_kept"));
    h.get_by_label(&t("export.finished.file_left"));
    assert!(shown(&h, "/out/customers.ndjson"));
}

#[test]
fn a_failed_result_says_so() {
    let h = harness(finished(Err(CoreError::Unsupported("x".into()))));
    h.get_by_label(&t("export.finished.failed"));
}

// ---- the entry point: the collection's context menu ----

use support::shots::{browsing, build_app};

const SIZE: (f32, f32) = (1000.0, 700.0);

/// The Browse page of the real app (in-memory store, crm.customers shown) with `adjust` applied
/// and, when given, `picker` answering the save dialog.
struct BrowsePage {
    harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

fn page_with(
    picker: Option<std::sync::Arc<dyn curlybrace_ui::app::file_picker::FilePicker>>,
    adjust: impl FnOnce(&mut curlybrace_ui::app::state::AppState),
) -> BrowsePage {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, dir) = build_app(vec![], None);
    if let Some(picker) = picker {
        app = app.with_file_picker(picker, std::sync::Arc::new(|| {}));
    }
    browsing(curlybrace_ui::viewmodel::ViewMode::Table)(app.state_mut());
    adjust(app.state_mut());
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(SIZE.0, SIZE.1))
        .with_pixels_per_point(1.0)
        .build_state(
            move |ctx, app: &mut CurlybraceApp| {
                if !installed {
                    curlybrace_ui::theme::install(ctx);
                    installed = true;
                }
                app.run_frame(ctx)
            },
            app,
        );
    harness.run_steps(6);
    drop(enter);
    BrowsePage {
        harness,
        _rt: rt,
        _dir: dir,
    }
}

fn page(adjust: impl FnOnce(&mut curlybrace_ui::app::state::AppState)) -> BrowsePage {
    page_with(None, adjust)
}

fn filtered(state: &mut curlybrace_ui::app::state::AppState) {
    state.page_query.filter = doc! { "status": "open" };
}

fn collated(state: &mut curlybrace_ui::app::state::AppState) {
    filtered(state);
    state.page_query.options.collation = Some(curlybrace_core::pipeline_options::CollationOption {
        locale: "en".into(),
        strength: None,
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    });
}

fn doc_export(page: &BrowsePage) -> &DocExportUi {
    &page.harness.state().state().doc_export
}

fn open_menu_of(page: &mut BrowsePage, collection: &str) {
    page.harness
        .get_by_role_and_label(Role::Button, collection)
        .click_secondary();
    page.harness.run_steps(3);
}

fn open_menu(page: &mut BrowsePage) {
    open_menu_of(page, "notes");
}

fn entry_state(page: &BrowsePage, key: &str) -> Option<bool> {
    let label = t(key);
    page.harness
        .query_by_role_and_label(Role::Button, &label)
        .map(|n| n.accesskit_node().is_disabled())
}

#[test]
fn the_sidebar_menu_exports_the_whole_collection() {
    let mut page = page(filtered);
    open_menu(&mut page);
    page.harness
        .get_by_role_and_label(Role::Button, &t("export.entry.menu"))
        .click();
    page.harness.run_steps(4);
    match doc_export(&page) {
        DocExportUi::Configuring { source, .. } => {
            assert_eq!(source.ns, Namespace::new("crm", "notes"));
            assert_eq!(source.filter, None);
        }
        other => panic!("not configuring: {other:?}"),
    }
    page.harness.get_by_label(&t("export.title"));
}

#[test]
fn a_collated_page_does_not_block_the_whole_collection_entry() {
    let mut page = page(collated);
    open_menu(&mut page);
    assert_eq!(entry_state(&page, "export.entry.menu"), Some(false));
}

#[test]
fn the_filter_entry_exports_what_the_filter_matches() {
    let mut page = page(filtered);
    open_menu_of(&mut page, "customers");
    assert_eq!(entry_state(&page, "export.entry.query"), Some(false));
    page.harness
        .get_by_role_and_label(Role::Button, &t("export.entry.query"))
        .click();
    page.harness.run_steps(4);
    match doc_export(&page) {
        DocExportUi::Configuring { source, .. } => {
            assert_eq!(source.ns, Namespace::new("crm", "customers"));
            assert_eq!(source.filter, Some(doc! { "status": "open" }));
        }
        other => panic!("not configuring: {other:?}"),
    }
    page.harness
        .get_by_label(&tw("export.source.query", &[("name", "crm.customers")]));
}

#[test]
fn the_filter_entry_is_absent_without_a_filter_or_on_another_collection() {
    let mut page = page(|_| {});
    open_menu_of(&mut page, "customers");
    assert_eq!(entry_state(&page, "export.entry.query"), None);
    assert_eq!(entry_state(&page, "export.entry.menu"), Some(false));
    let mut page = self::page(filtered);
    open_menu_of(&mut page, "notes");
    assert_eq!(entry_state(&page, "export.entry.query"), None);
}

#[test]
fn a_collation_closes_the_filter_entry_with_its_reason() {
    let mut page = page(collated);
    open_menu_of(&mut page, "customers");
    assert_eq!(entry_state(&page, "export.entry.query"), Some(true));
    let reason = t(DocExportBlock::Collated.i18n_key());
    assert!(shown_in(&page.harness, &reason));
}

#[test]
fn the_sidebar_entries_show_why_they_are_closed_while_an_export_runs() {
    // The tree is only drawn while connected, so the open reason is a running export.
    let mut page = page(filtered);
    open_menu_of(&mut page, "customers");
    app_of(&mut page).state_mut().doc_export = running(Some(10), 1, false);
    page.harness.run_steps(3);
    assert_eq!(entry_state(&page, "export.entry.menu"), Some(true));
    assert_eq!(entry_state(&page, "export.entry.query"), Some(true));
    let reason = t(DocExportBlock::Busy.i18n_key());
    assert!(shown_in(&page.harness, &reason));
}

fn shown_in(h: &Harness<'_, CurlybraceApp>, text: &str) -> bool {
    h.query_all_by_label(text).count() > 0
}

// ---- the whole flow, in the real app ----

fn app_of(page: &mut BrowsePage) -> &mut CurlybraceApp {
    page.harness.state_mut()
}

fn script(page: &mut BrowsePage, event: ExportEvent) {
    app_of(page).dispatch(Input::Event(Event::Export(event)));
}

#[test]
fn open_choose_and_finish_end_to_end() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = dir.path().join("customers.ndjson");
    let picker = support::export_flow::FakePicker::answering(vec![target.clone()]);
    let mut page = page_with(Some(picker.clone()), |_| {});

    open_menu(&mut page);
    page.harness
        .get_by_role_and_label(Role::Button, &t("export.entry.menu"))
        .click();
    page.harness.run_steps(4);
    page.harness.get_by_label(&t("export.title"));
    page.harness
        .get_by_role_and_label(Role::RadioButton, &t("export.format.json_array"))
        .click();
    page.harness.run_steps(3);
    page.harness
        .get_by_role_and_label(Role::Button, &t("export.choose_file"))
        .click();
    wait_until(&mut page.harness, "the export to start", SHORT, |h| {
        matches!(h.state().state().doc_export, DocExportUi::Running { .. })
    });
    // Both events go in before the next frame: the runtime has no session behind this app and
    // answers the start with a failure, which is stale once the run is over.
    let run = match doc_export(&page) {
        DocExportUi::Running { run, .. } => *run,
        other => panic!("not running: {other:?}"),
    };
    let progress = ExportEvent::Progress {
        run,
        done: 10,
        total: Some(10),
    };
    script(&mut page, progress);
    let done = ExportEvent::Done {
        run,
        outcome: outcome(10, false, false),
    };
    script(&mut page, done);
    page.harness.run_steps(4);
    page.harness
        .get_by_label(&tw("export.finished.done_many", &[("count", "10")]));
    assert!(
        page.harness.state().state().banner.is_none(),
        "the late failure is stale"
    );
    assert_eq!(
        picker.asked(),
        vec![("export".to_string(), "json".to_string())]
    );
    page.harness
        .get_by_role_and_label(Role::Button, &t("export.finished.close"))
        .click();
    page.harness.run_steps(3);
    assert_eq!(*doc_export(&page), DocExportUi::Idle);
}
