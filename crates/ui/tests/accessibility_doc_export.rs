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
        "export.running.progress",
        &[("done", "50"), ("total", "200")],
    ));
}

#[test]
fn an_unknown_total_counts_only_what_is_done() {
    let h = harness(running(None, 50, false));
    h.get_by_role_and_label(Role::ProgressIndicator, &t("export.running.bar_name"));
    h.get_by_label(&tw("export.running.progress_unknown", &[("done", "50")]));
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
    h.get_by_label(&tw("export.finished.done", &[("count", "42")]));
    assert!(!shown(&h, &t("export.finished.file_left")));
    h.get_by_role_and_label(Role::Button, &t("export.finished.close"))
        .click();
    h.run_steps(3);
    assert_eq!(actions(&h), vec![DocExportAction::Close]);
}

#[test]
fn a_cancelled_export_says_so() {
    let h = harness(finished(Ok(outcome(5, true, false))));
    h.get_by_label(&tw("export.finished.cancelled", &[("count", "5")]));
}

#[test]
fn a_left_file_asks_the_person_to_delete_it() {
    let h = harness(finished(Ok(outcome(5, true, true))));
    h.get_by_label(&t("export.finished.file_left"));
}

#[test]
fn a_failed_result_says_so() {
    let h = harness(finished(Err(CoreError::Unsupported("x".into()))));
    h.get_by_label(&t("export.finished.failed"));
}

// ---- the entry points ----

use curlybrace_ui::viewmodel::{BulkEntry, DocAction, TreeState, ViewMode};
use curlybrace_ui::views::documents::{DocTarget, show_documents};

fn toolbar(block: Option<DocExportBlock>) -> Harness<'static, (TreeState, Vec<DocAction>)> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 500.0))
        .build_ui_state(
            move |ui, (state, seen): &mut (TreeState, Vec<DocAction>)| {
                let target = DocTarget {
                    connection: "Vault",
                    production: false,
                    read_only: false,
                    bulk: BulkEntry::Available,
                    export: block,
                };
                let page = curlybrace_core::store::Page {
                    docs: vec![doc! { "_id": 1_i32 }],
                    skip: 0,
                    has_more: false,
                };
                let shown =
                    show_documents(ui, ViewMode::Tree, &page, state, target, &I18n::english());
                seen.extend(shown);
            },
            (TreeState::default(), Vec::new()),
        );
    h.run_steps(4);
    h
}

#[test]
fn the_toolbar_button_asks_to_export() {
    let mut h = toolbar(None);
    let label = t("export.entry.toolbar");
    let button = h.get_by_role_and_label(Role::Button, &label);
    assert!(!button.accesskit_node().is_disabled());
    button.click();
    h.run_steps(3);
    assert_eq!(h.state().1, vec![DocAction::Export]);
}

#[test]
fn a_closed_toolbar_button_shows_its_reason_as_text() {
    for block in [
        DocExportBlock::NotConnected,
        DocExportBlock::Collated,
        DocExportBlock::Busy,
    ] {
        let mut h = toolbar(Some(block));
        let reason = t(block.i18n_key());
        let label = t("export.entry.toolbar");
        let button = h.get_by_role_and_label(Role::Button, &label);
        assert!(button.accesskit_node().is_disabled(), "{block:?}");
        assert_eq!(
            button.accesskit_node().description().as_deref(),
            Some(reason.as_str()),
            "{block:?}"
        );
        h.get_by_label(&reason);
        h.get_by_role_and_label(Role::Button, &t("export.entry.toolbar"))
            .click();
        h.run_steps(3);
        assert!(h.state().1.is_empty(), "{block:?} emits nothing");
    }
}

use support::browse_page::{BrowsePage, SIZE_1000, browse_page};

fn page(adjust: impl FnOnce(&mut curlybrace_ui::app::state::AppState)) -> BrowsePage {
    browse_page(SIZE_1000, adjust)
}

fn doc_export(page: &BrowsePage) -> &DocExportUi {
    &page.harness.state().state().doc_export
}

fn open_menu(page: &mut BrowsePage) {
    page.harness
        .get_by_role_and_label(Role::Button, "notes")
        .click_secondary();
    page.harness.run_steps(3);
}

#[test]
fn the_sidebar_menu_exports_the_whole_collection() {
    let mut page = page(|_| {});
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
fn a_collated_page_does_not_block_the_sidebar_entry() {
    let mut page = page(|state| {
        state.page_query.options.collation =
            Some(curlybrace_core::pipeline_options::CollationOption {
                locale: "en".into(),
                strength: None,
                case_level: None,
                case_first: None,
                numeric_ordering: None,
                alternate: None,
            });
    });
    open_menu(&mut page);
    let label = t("export.entry.menu");
    let entry = page.harness.get_by_role_and_label(Role::Button, &label);
    assert!(!entry.accesskit_node().is_disabled());
}

#[test]
fn the_sidebar_entry_shows_why_it_is_closed() {
    // The tree is only drawn while connected, so the open reason is a running export.
    let mut page = page(|_| {});
    open_menu(&mut page);
    app_of(&mut page).state_mut().doc_export = running(Some(10), 1, false);
    page.harness.run_steps(3);
    let label = t("export.entry.menu");
    let entry = page.harness.get_by_role_and_label(Role::Button, &label);
    assert!(entry.accesskit_node().is_disabled());
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
    let mut page = support::browse_page::browse_page_with_picker(SIZE_1000, picker.clone());

    page.harness
        .get_by_role_and_label(Role::Button, &t("export.entry.toolbar"))
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
        .get_by_label(&tw("export.finished.done", &[("count", "10")]));
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
