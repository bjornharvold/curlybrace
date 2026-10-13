//! Click-driven Compare FIELD-LEVEL apply flows: the REAL app and runtime against a real
//! MongoDB. The gestures are opening a document's detail, ticking fields by their path, "Copy
//! fields to ...", Review changes, Apply; the assertions read the screen and the database. Tier 1
//! like `local_compare_apply_flows.rs`, and like it these tests WRITE to the compared collections:
//! run them against a throwaway server of your own (see docs/superpowers/notes/testing.md). Only
//! `cbit_*` databases are used and the fixture drops them.
mod support;

use bson::{Document, doc};
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::viewmodel::compare::{ApplySummary, RulesForm, ScopeKind, StatusIcon};
use egui_kittest::kittest::Queryable;
use support::actions::*;
use support::compare::*;
use support::fixture::Fixture;
use support::wait::SHORT;

/// The name of the enclosing test function (for the printed skip reason).
macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

macro_rules! fixture {
    () => {{
        // These flows write: never fall back to the default server.
        if std::env::var_os("CURLYBRACE_IT").is_some()
            && !std::env::var("CURLYBRACE_IT_MONGODB_URI").is_ok_and(|v| !v.trim().is_empty())
        {
            eprintln!(
                "SKIPPED {}: set CURLYBRACE_IT_MONGODB_URI to a throwaway server (not 27017)",
                test_name!()
            );
            return;
        }
        let Some(f) = Fixture::local(test_name!()) else {
            return;
        };
        f
    }};
}

/// One pair `left` -> `right` of the test database, checked and run; its results are open.
#[track_caller]
fn compared(f: &mut Fixture) {
    open_compare(f);
    pick_pair_and_check(f, "left", "right");
    start_and_wait(f, SHORT);
    wait_shows_key(f, "compare.results.back_to_overview");
}

fn apply_summary(f: &Fixture) -> ApplySummary {
    let apply = &f.harness.state().state().compare.apply;
    apply.summary().expect("the apply finished")
}

fn one(f: &Fixture, collection: &str, id: i32) -> Document {
    f.db.find_one(collection, doc! {"_id": id})
        .unwrap_or_else(|| panic!("{collection} has no document {id}"))
}

/// Opens the Different tab (`count` rows) and the detail of the document `key`.
#[track_caller]
fn open_different(f: &mut Fixture, count: usize, key: &str) {
    open_tab(f, "compare.category.different", count);
    open_detail(f, key);
}

fn pending_items(f: &Fixture) -> usize {
    f.harness.state().state().compare.pending.items().len()
}

fn skipped_label(f: &Fixture, count: &str) -> String {
    let text = f
        .labels
        .t_with("compare.apply.outcome.stale", &[("count", count)]);
    format!("{} {text}", StatusIcon::Warning.glyph())
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn copy_one_field_to_the_target_and_leave_the_rest() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![
            doc! {"_id": 1_i32, "a": 11_i32, "b": 12_i32},
            doc! {"_id": 2_i32, "a": 5_i32},
        ],
    );
    f.db.insert(
        "right",
        vec![
            doc! {"_id": 1_i32, "a": 21_i32, "b": 22_i32},
            doc! {"_id": 2_i32, "a": 5_i32},
        ],
    );
    compared(&mut f);

    open_different(&mut f, 1, "1");
    tick_fields(&mut f, &["a"]);
    assert!(field_ticked(&f, "a") && !field_ticked(&f, "b"));
    apply_ticked_fields(&mut f, true);

    let summary = apply_summary(&f);
    assert_eq!((summary.applied, summary.failed, summary.stale), (1, 0, 0));
    assert_eq!(
        one(&f, "right", 1),
        doc! {"_id": 1_i32, "a": 11_i32, "b": 22_i32},
        "only the ticked field was copied"
    );
    assert_eq!(
        one(&f, "left", 1),
        doc! {"_id": 1_i32, "a": 11_i32, "b": 12_i32},
        "the source is untouched"
    );
    dismiss_result(&mut f);
    let rerun = rerun(&mut f, "left");
    assert_eq!(
        (rerun.equal, rerun.different),
        (1, 1),
        "the document still differs in the field that was not copied"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn copy_two_fields_of_one_document_in_one_apply_and_rerun_shows_the_rest() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![doc! {"_id": 1_i32, "a": 11_i32, "b": 12_i32, "c": 13_i32}],
    );
    f.db.insert(
        "right",
        vec![doc! {"_id": 1_i32, "a": 21_i32, "b": 22_i32, "c": 23_i32}],
    );
    compared(&mut f);

    open_different(&mut f, 1, "1");
    tick_fields(&mut f, &["a", "c"]);
    apply_ticked_fields(&mut f, true);

    let summary = apply_summary(&f);
    assert_eq!(
        (summary.applied, summary.failed, summary.stale),
        (1, 0, 0),
        "two fields of one document are one write"
    );
    assert_eq!(
        one(&f, "right", 1),
        doc! {"_id": 1_i32, "a": 11_i32, "b": 22_i32, "c": 13_i32}
    );
    dismiss_result(&mut f);
    let rerun = rerun(&mut f, "left");
    assert_eq!((rerun.equal, rerun.different), (0, 1));
    // The re-run lists exactly the field that is left.
    open_tab(&mut f, "compare.category.different", 1);
    open_detail(&mut f, "1");
    assert!(!field_disabled(&f, "b"), "b is the field that is left");
    assert!(!has_field(&f, "a"), "a is equal now");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn remove_a_field_that_only_the_target_has() {
    let mut f = fixture!();
    f.db.insert("left", vec![doc! {"_id": 1_i32, "a": 11_i32}]);
    f.db.insert(
        "right",
        vec![doc! {"_id": 1_i32, "a": 11_i32, "extra": 99_i32, "keep": 7_i32}],
    );
    compared(&mut f);

    open_different(&mut f, 1, "1");
    assert!(!field_disabled(&f, "extra"), "removing a field is allowed");
    tick_fields(&mut f, &["extra"]);
    apply_ticked_fields(&mut f, true);

    assert_eq!(apply_summary(&f).applied, 1);
    assert_eq!(
        one(&f, "right", 1),
        doc! {"_id": 1_i32, "a": 11_i32, "keep": 7_i32},
        "the field is unset on the target, nothing else"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn remove_a_field_that_only_the_source_has_by_copying_to_the_source() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![doc! {"_id": 1_i32, "a": 11_i32, "extra": 99_i32, "keep": 7_i32}],
    );
    f.db.insert("right", vec![doc! {"_id": 1_i32, "a": 11_i32}]);
    compared(&mut f);

    open_different(&mut f, 1, "1");
    tick_fields(&mut f, &["extra"]);
    apply_ticked_fields(&mut f, false);

    assert_eq!(apply_summary(&f).applied, 1);
    assert_eq!(
        one(&f, "left", 1),
        doc! {"_id": 1_i32, "a": 11_i32, "keep": 7_i32},
        "the field is unset on the source"
    );
    assert_eq!(
        one(&f, "right", 1),
        doc! {"_id": 1_i32, "a": 11_i32},
        "the target is untouched"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_field_in_a_nested_document_and_an_array_element_by_index() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![doc! {
            "_id": 1_i32,
            "address": {"city": "Oslo", "zip": "0150"},
            "tags": ["x", "y", "z-left"],
        }],
    );
    f.db.insert(
        "right",
        vec![doc! {
            "_id": 1_i32,
            "address": {"city": "Bergen", "zip": "5003"},
            "tags": ["x", "q", "z-right"],
        }],
    );
    compared(&mut f);

    open_different(&mut f, 1, "1");
    assert!(!field_disabled(&f, "tags[1]"), "a changed element by index");
    tick_fields(&mut f, &["address.city", "tags[1]"]);
    apply_ticked_fields(&mut f, true);

    assert_eq!(apply_summary(&f).applied, 1);
    let right = one(&f, "right", 1);
    assert_eq!(
        right,
        doc! {
            "_id": 1_i32,
            "address": {"city": "Oslo", "zip": "5003"},
            "tags": ["x", "y", "z-right"],
        },
        "the nested field and the element at index 1 changed, nothing around them"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_document_changed_after_the_compare_is_reported_stale_and_left_alone() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![
            doc! {"_id": 1_i32, "a": 11_i32},
            doc! {"_id": 2_i32, "a": 12_i32},
        ],
    );
    f.db.insert(
        "right",
        vec![
            doc! {"_id": 1_i32, "a": 21_i32},
            doc! {"_id": 2_i32, "a": 22_i32},
        ],
    );
    compared(&mut f);

    open_tab(&mut f, "compare.category.different", 2);
    for key in ["1", "2"] {
        open_detail(&mut f, key);
        tick_fields(&mut f, &["a"]);
        copy_fields(&mut f, true);
    }
    assert_eq!(pending_items(&f), 2, "one item per document");
    open_review(&mut f);
    // A second client changes a field of one target document between the compare and the apply.
    f.db.set_fields("right", doc! {"_id": 1_i32}, doc! {"other": 999_i32});
    confirm_and_wait(&mut f, SHORT);

    let summary = apply_summary(&f);
    assert_eq!((summary.applied, summary.stale, summary.failed), (1, 1, 0));
    let skipped = skipped_label(&f, "1");
    wait_shows(&mut f, &skipped);
    assert_eq!(
        one(&f, "right", 1),
        doc! {"_id": 1_i32, "a": 21_i32, "other": 999_i32},
        "the changed document is left exactly as the second client wrote it"
    );
    assert_eq!(one(&f, "right", 2), doc! {"_id": 2_i32, "a": 12_i32});
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_field_with_a_dot_in_its_name_is_shown_disabled_and_cannot_be_queued() {
    let mut f = fixture!();
    // MongoDB 5.0 and later store field names with dots; the driver sends them as they are.
    let mut left = doc! {"_id": 1_i32, "n": 11_i32};
    left.insert("a.b", 1_i32);
    let mut right = doc! {"_id": 1_i32, "n": 21_i32};
    right.insert("a.b", 2_i32);
    f.db.insert("left", vec![left]);
    f.db.insert("right", vec![right.clone()]);
    compared(&mut f);

    open_different(&mut f, 1, "1");
    assert!(
        field_disabled(&f, "a.b"),
        "a dotted name is not addressable"
    );
    assert!(!field_disabled(&f, "n"));
    let reason = f.labels.t("compare.apply.field_reason.bad_name");
    wait_shows(&mut f, &reason);
    // Clicking the disabled checkbox ticks nothing; with nothing ticked there is nothing to queue.
    tick_fields(&mut f, &["a.b"]);
    assert!(!field_ticked(&f, "a.b"));
    assert!(copy_fields_disabled(&f, true) && copy_fields_disabled(&f, false));
    assert_eq!(pending_items(&f), 0);
    // The other field of the same document is still copyable on its own.
    tick_fields(&mut f, &["n"]);
    apply_ticked_fields(&mut f, true);
    assert_eq!(apply_summary(&f).applied, 1);
    let mut expected = right;
    expected.insert("n", 11_i32);
    assert_eq!(
        one(&f, "right", 1),
        expected,
        "the dotted field was not touched"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn field_and_whole_document_changes_for_different_documents_in_one_apply() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![
            doc! {"_id": 1_i32, "a": 11_i32, "b": 12_i32},
            doc! {"_id": 2_i32, "a": 15_i32, "b": 16_i32},
        ],
    );
    f.db.insert(
        "right",
        vec![
            doc! {"_id": 1_i32, "a": 21_i32, "b": 22_i32},
            doc! {"_id": 2_i32, "a": 25_i32, "b": 26_i32},
        ],
    );
    compared(&mut f);

    open_tab(&mut f, "compare.category.different", 2);
    open_detail(&mut f, "1");
    tick_fields(&mut f, &["a"]);
    copy_fields(&mut f, true);
    assert_eq!(pending_items(&f), 1);
    tick_rows(&mut f, &["2"]);
    // The open detail has its own "Copy to target" for document 1; the bar of the tab acts on
    // the ticked rows (document 2). Both are on screen: the one that queues a second item wins.
    let label = f.labels.t("compare.apply.action.copy_to_target");
    let buttons = f.harness.get_all_by_label(&label).count();
    assert_eq!(buttons, 2, "the tab's bar and the detail pane");
    f.harness.get_all_by_label(&label).next().unwrap().click();
    support::wait::settle(&mut f.harness);
    assert_eq!(pending_items(&f), 2, "a Fields item and a Replace item");
    open_review(&mut f);
    confirm_and_wait(&mut f, SHORT);

    let summary = apply_summary(&f);
    assert_eq!(
        (
            summary.applied,
            summary.failed,
            summary.stale,
            summary.not_run
        ),
        (2, 0, 0, 0),
        "both documents applied, counts add up: {summary:?}"
    );
    assert_eq!(
        one(&f, "right", 1),
        doc! {"_id": 1_i32, "a": 11_i32, "b": 22_i32},
        "document 1: one field"
    );
    assert_eq!(
        one(&f, "right", 2),
        one(&f, "left", 2),
        "document 2: replaced"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn field_scope_only_never_writes_outside_the_scope() {
    let mut f = fixture!();
    f.db.insert(
        "left",
        vec![doc! {"_id": 1_i32, "a": 11_i32, "b": 12_i32, "c": 13_i32}],
    );
    f.db.insert(
        "right",
        vec![doc! {"_id": 1_i32, "a": 21_i32, "b": 22_i32, "c": 23_i32}],
    );
    open_compare(&mut f);
    // The rules apply to the pairs added after this.
    let rules = RulesForm {
        scope: ScopeKind::Only,
        paths_text: "a".to_string(),
        ..RulesForm::default()
    };
    send(&mut f, CompareAction::SetRules(rules));
    pick_pair_and_check(&mut f, "left", "right");
    start_and_wait(&mut f, SHORT);
    wait_shows_key(&mut f, "compare.results.back_to_overview");

    open_different(&mut f, 1, "1");
    assert!(
        !has_field(&f, "b"),
        "fields outside the scope are not offered"
    );
    tick_fields(&mut f, &["a"]);
    apply_ticked_fields(&mut f, true);

    assert_eq!(apply_summary(&f).applied, 1);
    assert_eq!(
        one(&f, "right", 1),
        doc! {"_id": 1_i32, "a": 11_i32, "b": 22_i32, "c": 23_i32},
        "b and c differ but are outside the compared scope"
    );
    dismiss_result(&mut f);
    let rerun = rerun(&mut f, "left");
    assert_eq!(
        (rerun.equal, rerun.different),
        (1, 0),
        "equal within the scope"
    );
}
