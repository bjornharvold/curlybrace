//! Click-driven undo of a bulk write: the REAL app and runtime, driven through the accessibility
//! tree by `egui_kittest`, against a real MongoDB. The screen and the database are both asserted.
//!
//! Needs a local MongoDB (`CURLYBRACE_IT=1`, `CURLYBRACE_IT_MONGODB_URI`); see
//! `local_e2e_flows.rs`. Every test writes to its own throwaway `cbit_*` database.
mod support;

use bson::doc;
use egui_kittest::kittest::Queryable;
use support::actions::*;
use support::db::people;
use support::fixture::Fixture;

const PEOPLE: &str = "people";

macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

/// Types the collection name into the confirmation box captioned for it.
fn type_collection_name(f: &mut Fixture) {
    let caption = f
        .labels
        .t_with("bulk.type_collection_name", &[("name", PEOPLE)]);
    // Let the layout settle (a just-arrived preview moves the field for a frame or two).
    f.harness.run_steps(6);
    fill(f, &caption, PEOPLE);
}

/// Clicks the lowest button labelled `label`: the dialog's footer button, under the page's own
/// button of the same label.
fn click_lowest(f: &mut Fixture, label: &str) {
    let target = f
        .harness
        .get_all_by_label(label)
        .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
        .expect("a button with that label");
    target.click();
    support::wait::settle(&mut f.harness);
}

/// Delete matching -> Define -> Preview -> Confirm. The dialog's Next shares its label with the
/// page's pagination button, so it is the one below the step heading.
fn to_the_confirm_step(f: &mut Fixture) {
    click_key(f, "bulk.delete_matching");
    for step in ["bulk.step_define", "bulk.step_preview"] {
        let heading = f.labels.t(step);
        wait_shows(f, &heading);
        // Next is off until the preview has loaded.
        let pending = f.labels.t("bulk.preview_pending");
        wait_gone(f, &pending);
        let next = f.labels.t("bulk.next");
        click_lowest(f, &next);
    }
    let heading = f.labels.t("bulk.step_confirm");
    wait_shows(f, &heading);
    let count = f.labels.t_with("bulk.count_known", &[("n", "3")]);
    wait_shows(f, &count);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn deleting_every_document_is_undone_through_the_dialog() {
    let Some(mut f) = Fixture::local(test_name!()) else {
        return;
    };
    f.db.insert(PEOPLE, people());
    browse(&mut f, PEOPLE);

    // Define -> Preview -> Confirm, as a person would.
    to_the_confirm_step(&mut f);

    // The dialog says where the copy goes before anything is written.
    let notice_start = f.labels.t("bulk.undo.copy_notice");
    let prefix = notice_start
        .split('{')
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(
        f.harness
            .query_all_by_label_contains(&prefix)
            .next()
            .is_some(),
        "the undo copy notice is shown before the write"
    );

    let tick = f
        .labels
        .t_with("bulk.confirm.all_documents_tick", &[("collection", PEOPLE)]);
    f.click_text(&tick);
    type_collection_name(&mut f);
    click_key(&mut f, "bulk.run_delete");

    let offered = f.labels.t_with("bulk.undo.available", &[("n", "3")]);
    wait_shows(&mut f, &offered);
    assert_eq!(f.db.count(PEOPLE), 0, "the write ran");

    // Undo: preview, typed name, run.
    click_key(&mut f, "bulk.undo.open");
    let heading = f.labels.t("bulk.undo.preview_heading");
    wait_shows(&mut f, &heading);
    type_collection_name(&mut f);
    click_key(&mut f, "bulk.undo.run");

    let restored = f.labels.t_with("bulk.undo.report.restored", &[("n", "3")]);
    wait_shows(&mut f, &restored);

    let mut after = f.db.all(PEOPLE);
    after.sort_by_key(|d| d.get_i32("_id").unwrap_or_default());
    assert_eq!(after, people(), "every deleted document is back, unchanged");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_document_changed_since_the_write_is_left_alone_by_undo() {
    let Some(mut f) = Fixture::local(test_name!()) else {
        return;
    };
    f.db.insert(PEOPLE, people());
    browse(&mut f, PEOPLE);

    to_the_confirm_step(&mut f);
    let tick = f
        .labels
        .t_with("bulk.confirm.all_documents_tick", &[("collection", PEOPLE)]);
    f.click_text(&tick);
    type_collection_name(&mut f);
    click_key(&mut f, "bulk.run_delete");
    let offered = f.labels.t_with("bulk.undo.available", &[("n", "3")]);
    wait_shows(&mut f, &offered);

    // Someone recreates one of the documents with different content.
    f.db.insert(PEOPLE, vec![doc! {"_id": 2_i32, "name": "Somebody Else"}]);

    click_key(&mut f, "bulk.undo.open");
    let heading = f.labels.t("bulk.undo.preview_heading");
    wait_shows(&mut f, &heading);
    type_collection_name(&mut f);
    click_key(&mut f, "bulk.undo.run");

    let restored = f.labels.t_with("bulk.undo.report.restored", &[("n", "2")]);
    wait_shows(&mut f, &restored);
    let skipped = f
        .labels
        .t_with("bulk.undo.report.skipped_present", &[("n", "1")]);
    wait_shows(&mut f, &skipped);
    let kept = f.db.find_one(PEOPLE, doc! {"_id": 2_i32}).expect("kept");
    assert_eq!(kept.get_str("name").unwrap(), "Somebody Else");
    assert_eq!(f.db.count(PEOPLE), 3);
}
