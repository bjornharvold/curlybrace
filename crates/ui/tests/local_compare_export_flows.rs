//! Click-driven Compare EXPORT flows: the REAL app and runtime against a real MongoDB, with a
//! fake file dialog that answers with a path in a temp directory. Tier 1 like
//! `local_compare_apply_flows.rs`: it needs `CURLYBRACE_IT=1` and an explicit
//! `CURLYBRACE_IT_MONGODB_URI` (a throwaway server of your own, see
//! docs/superpowers/notes/testing.md). An export never writes to the databases, and every test
//! asserts exactly that. Only `cbit_*` databases are used and the fixture drops them.
mod support;

use bson::{Document, doc};
use curlybrace_ui::viewmodel::compare::ExportKind;
use support::actions::*;
use support::compare::*;
use support::export_flow::*;
use support::fixture::Fixture;
use support::screen::shows;
use support::wait::SHORT;

macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

/// A fixture whose save dialog answers with `paths`, or None (printing why) when not enabled.
macro_rules! fixture {
    ($picker:expr) => {{
        if std::env::var_os("CURLYBRACE_IT").is_some()
            && !std::env::var("CURLYBRACE_IT_MONGODB_URI").is_ok_and(|v| !v.trim().is_empty())
        {
            eprintln!(
                "SKIPPED {}: set CURLYBRACE_IT_MONGODB_URI to a throwaway server (not 27017)",
                test_name!()
            );
            return;
        }
        let Some(f) = Fixture::local_with_picker(test_name!(), $picker) else {
            return;
        };
        f
    }};
}

const SECRET: &str = "SENTINEL-DOC-VALUE-9f3a";

/// Both collections as the databases hold them: nothing an export does may change this.
fn snapshot(f: &Fixture) -> (Vec<Document>, Vec<Document>) {
    (f.db.all("left"), f.db.all("right"))
}

/// left: 1..=4 (3 only here); right: 2 differs, 4 equal, 9 only there.
fn seed(f: &Fixture) {
    f.db.insert(
        "left",
        vec![
            doc! { "_id": 1, "n": 1, "secret": SECRET },
            doc! { "_id": 2, "n": 2, "secret": SECRET },
            doc! { "_id": 3, "n": 3 },
            doc! { "_id": 4, "n": 4 },
        ],
    );
    f.db.insert(
        "right",
        vec![
            doc! { "_id": 2, "n": 20 },
            doc! { "_id": 4, "n": 4 },
            doc! { "_id": 9, "n": 9 },
        ],
    );
}

fn compared(f: &mut Fixture) {
    open_compare(f);
    pick_pair_and_check(f, "left", "right");
    start_and_wait(f, SHORT);
    wait_shows_key(f, "compare.results.back_to_overview");
}

/// Queues the two source-only inserts and the one replace.
fn queue_changes(f: &mut Fixture) {
    open_tab(f, "compare.category.only_in_source", 2);
    select_all_in_tab(f, 2);
    click_action(f, "compare.apply.action.copy_to_target");
    open_tab(f, "compare.category.different", 1);
    select_all_in_tab(f, 1);
    click_action(f, "compare.apply.action.copy_to_target");
}

fn count_text(f: &Fixture, key: &str, n: u64) -> String {
    f.labels.t_with(key, &[("count", &n.to_string())])
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn export_a_plan_from_the_pending_list_writes_only_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let (script, plan) = (
        dir.path().join("changes.js"),
        dir.path().join("changes.json"),
    );
    let picker = FakePicker::answering(vec![script.clone(), plan.clone()]);
    let mut f = fixture!(picker.clone());
    seed(&f);
    compared(&mut f);
    queue_changes(&mut f);
    let before = snapshot(&f);

    // A mongosh script (the default format).
    open_export(&mut f, ExportKind::Plan);
    let warning = f.labels.t("compare.export.warning.data");
    assert!(shows(&f.harness, &warning), "the data warning is shown");
    fill_key(&mut f, "compare.export.label.caption", "release 42");
    choose_file_and_wait(&mut f);
    let outcome = finished(&f);
    assert_eq!(
        (
            outcome.written,
            outcome.skipped,
            outcome.failed,
            outcome.not_run
        ),
        (3, 0, 0, 0),
        "{outcome:?}"
    );
    for (key, n) in [
        ("compare.export.finished.written", 3),
        ("compare.export.finished.skipped", 0),
        ("compare.export.finished.failed", 0),
    ] {
        let text = count_text(&f, key, n);
        wait_shows(&mut f, &text);
    }
    let path_text = script.to_string_lossy().into_owned();
    assert!(shows(&f.harness, &path_text), "the chosen path is shown");
    let text = std::fs::read_to_string(&script).unwrap();
    assert!(text.starts_with("// complete export"), "{text}");
    assert!(text.contains("release 42"));
    assert_eq!(text.matches("insertOne").count(), 2);
    assert_eq!(text.matches("replaceOne").count(), 1);
    assert_eq!(snapshot(&f), before, "an export writes nothing");
    assert_eq!(
        picker.asked(),
        vec![("curlybrace-plan.js".to_string(), "js".to_string())]
    );
    dismiss(&mut f);

    // The same pending list as a JSON plan; the list itself is still there.
    open_export(&mut f, ExportKind::Plan);
    click_key(&mut f, "compare.export.format.json");
    choose_file_and_wait(&mut f);
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).unwrap()).expect("valid JSON");
    assert_eq!(parsed["header"]["format"], "curlybrace-compare-plan");
    assert_eq!(parsed["header"]["complete"], true);
    let ops = parsed["pairs"][0]["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 3);
    assert_eq!(snapshot(&f), before, "still nothing written");
    assert_eq!(picker.asked().len(), 2);
    assert_eq!(picker.asked()[1].0, "curlybrace-plan.json");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "no other file in the directory"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn export_a_report_from_the_overview_writes_only_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.csv");
    let picker = FakePicker::answering(vec![report.clone()]);
    let mut f = fixture!(picker.clone());
    seed(&f);
    compared(&mut f);
    click_key(&mut f, "compare.results.back_to_overview");
    let before = snapshot(&f);

    open_export(&mut f, ExportKind::Report);
    let keys_warning = f.labels.t("compare.export.warning.report_keys");
    assert!(shows(&f.harness, &keys_warning));
    choose_file_and_wait(&mut f);
    let outcome = finished(&f);
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (4, 0, 0),
        "{outcome:?}"
    );
    let rows = count_text(&f, "compare.export.finished.report_rows", 4);
    wait_shows(&mut f, &rows);
    let csv = std::fs::read_to_string(&report).unwrap();
    assert!(
        csv.contains("pair,source,target,category,key,changed_paths"),
        "{csv}"
    );
    assert!(csv.contains("# end of report: 4 rows"), "{csv}");
    for category in ["only_in_source", "different", "only_in_target"] {
        assert!(csv.contains(category), "{csv}");
    }
    assert!(!csv.contains(SECRET), "a report lists keys, never values");
    assert_eq!(snapshot(&f), before, "an export writes nothing");
    assert_eq!(
        picker.asked(),
        vec![("curlybrace-report.csv".to_string(), "csv".to_string())]
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_cancelled_file_dialog_exports_nothing() {
    let picker = std::sync::Arc::new(FakePicker::default());
    let mut f = fixture!(picker.clone());
    seed(&f);
    compared(&mut f);
    click_key(&mut f, "compare.results.back_to_overview");
    let before = snapshot(&f);
    open_export(&mut f, ExportKind::Report);
    click_key(&mut f, "compare.export.choose_file");
    // The dialog answered "cancelled": the modal is back with its options and no export ran.
    let choose = f.labels.t("compare.export.choose_file");
    wait_shows(&mut f, &choose);
    assert_eq!(picker.asked().len(), 1);
    assert_eq!(snapshot(&f), before);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn the_clipboard_report_is_copied_once_and_no_file_is_made() {
    let dir = tempfile::tempdir().unwrap();
    let picker = std::sync::Arc::new(FakePicker::default());
    let mut f = fixture!(picker.clone());
    seed(&f);
    compared(&mut f);
    click_key(&mut f, "compare.results.back_to_overview");
    let before = snapshot(&f);

    open_export(&mut f, ExportKind::Report);
    let copied = copy_report(&mut f);
    assert_eq!(copied.len(), 1, "copied exactly once: {copied:?}");
    assert!(copied[0].contains("pair,source,target,category,key,changed_paths"));
    assert!(copied[0].contains("# end of report: 4 rows"));
    assert!(!copied[0].contains(SECRET));
    let rows = count_text(&f, "compare.export.copied.rows", 4);
    wait_shows(&mut f, &rows);
    assert!(
        picker.asked().is_empty(),
        "the clipboard needs no file dialog"
    );
    assert_eq!(snapshot(&f), before);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}
