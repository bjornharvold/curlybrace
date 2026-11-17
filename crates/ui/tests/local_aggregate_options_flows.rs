//! Click-driven flows of the Aggregate tab's Options section: the REAL app and runtime, driven
//! through the accessibility tree by `egui_kittest`, against a real MongoDB. Tier 1 (a local
//! server): every test writes to its own throwaway `cbit_*` database and drops it. Run with
//! `scripts/integration-tests.sh`, and ALWAYS point `CURLYBRACE_IT_MONGODB_URI` at a throwaway
//! server (the default is a developer's own MongoDB on 27017).
mod support;

use bson::doc;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use support::actions::*;
use support::fixture::Fixture;
use support::wait::{SHORT, settle, wait_until};

const WORDS: &str = "words";

/// The name of the enclosing test function (for the printed skip reason).
macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

macro_rules! fixture {
    () => {{
        let Some(f) = Fixture::local(test_name!()) else {
            return;
        };
        f.db.insert(
            WORDS,
            vec![
                doc! {"_id": 1_i32, "name": "abc"},
                doc! {"_id": 2_i32, "name": "ABC"},
                doc! {"_id": 3_i32, "name": "xyz"},
            ],
        );
        f
    }};
}

fn shown(f: &Fixture, text: &str) -> bool {
    f.harness.query_all_by_label_contains(text).next().is_some()
}

/// The Aggregate tab of `words` with one `$match` on `name: "abc"`, previewed.
fn matching_abc(f: &mut Fixture) {
    browse(f, WORDS);
    click_key(f, "aggregate.tab");
    let add = f.labels.t("aggregate.add_stage");
    f.harness
        .get_by_role_and_label(Role::ComboBox, &add)
        .click();
    settle(&mut f.harness);
    f.click_text("$match");
    let editor = f.labels.t_with("aggregate.stage.editor_n", &[("n", "1")]);
    fill(f, &editor, r#"{ "$match": { "name": "abc" } }"#);
    // The new stage starts as `{}` (every document): wait for the narrowed preview.
    wait_until(&mut f.harness, "the one matching document", SHORT, |h| {
        h.query_all_by_label_contains("abc").next().is_some()
            && h.query_all_by_label_contains("xyz").next().is_none()
    });
}

fn open_options(f: &mut Fixture) {
    click_key(f, "aggregate.options.toggle");
}

fn pick(f: &mut Fixture, combo_key: &str, item: &str) {
    let label = f.labels.t(combo_key);
    f.harness
        .get_by_role_and_label(Role::ComboBox, &label)
        .click();
    settle(&mut f.harness);
    f.click_text(item);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_collation_set_in_the_section_changes_the_preview() {
    let mut f = fixture!();
    matching_abc(&mut f);
    assert!(!shown(&f, "ABC"), "case matters without a collation");
    open_options(&mut f);
    click_key(&mut f, "aggregate.options.collation.enabled");
    fill_key(&mut f, "aggregate.options.collation.locale", "en");
    let second = f.labels.t("aggregate.options.collation.strength_value.2");
    pick(&mut f, "aggregate.options.collation.strength", &second);
    wait_until(
        &mut f.harness,
        "ABC beside abc under strength 2",
        SHORT,
        |h| h.query_all_by_label_contains("ABC").next().is_some(),
    );
    assert!(shown(&f, "abc"));
    assert_eq!(f.db.count(WORDS), 3, "a preview never writes");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_hint_on_a_missing_index_says_so_in_the_preview() {
    let mut f = fixture!();
    matching_abc(&mut f);
    open_options(&mut f);
    fill_key(&mut f, "aggregate.options.hint.label", "nope_1");
    let said = f.labels.t("errors.hint_not_found");
    wait_until(&mut f.harness, "the hint error", SHORT, |h| {
        h.query_all_by_label_contains(&said).next().is_some()
    });
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn the_hint_combo_lists_the_collection_s_indexes() {
    let mut f = fixture!();
    f.db.create_unique_index(WORDS, "name");
    matching_abc(&mut f);
    open_options(&mut f);
    let existing = f.labels.t("aggregate.options.hint.existing");
    wait_until(&mut f.harness, "the index list", SHORT, |h| {
        h.query_all_by_role_and_label(Role::ComboBox, &existing)
            .next()
            .is_some()
    });
    f.harness
        .get_by_role_and_label(Role::ComboBox, &existing)
        .click();
    settle(&mut f.harness);
    assert!(shown(&f, "name_1"), "{}", dump_screen(&f));
    f.click_text("name_1");
    wait_until(&mut f.harness, "the hint set to the index", SHORT, |h| {
        h.query_all_by_label_contains("hint: name_1")
            .next()
            .is_some()
    });
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_saved_pipeline_reopens_with_its_options() {
    let mut f = fixture!();
    matching_abc(&mut f);
    open_options(&mut f);
    fill_key(&mut f, "aggregate.options.comment.label", "e2e comment");
    let conn = f.saved_id();
    // Save under a name.
    click_key(&mut f, "aggregate.saved.save");
    fill_key(&mut f, "aggregate.saved.name_label", "with options");
    let save = f.labels.t("aggregate.saved.save");
    let buttons: Vec<_> = f
        .harness
        .query_all_by_role_and_label(Role::Button, &save)
        .map(|n| n.rect())
        .collect();
    assert!(!buttons.is_empty());
    f.harness
        .query_all_by_role_and_label(Role::Button, &save)
        .last()
        .expect("the dialog's Save")
        .click();
    wait_until(&mut f.harness, "the saved pipeline", SHORT, |h| {
        !h.state().state().pipelines.list(conn).is_empty()
    });
    let stored = f.harness.state().state().pipelines.list(conn)[0].clone();
    assert_eq!(stored.options.comment.as_deref(), Some("e2e comment"));
    // Change the options, then open the saved pipeline again: they come back.
    let on = f.labels.t("aggregate.options.tri.on");
    pick(&mut f, "aggregate.options.allow_disk_use.label", &on);
    wait_until(&mut f.harness, "disk use set", SHORT, |h| {
        h.state().state().aggregate.options.allow_disk_use == Some(true)
    });
    let open = f.labels.t("aggregate.saved.open");
    f.harness.get_by_role_and_label(Role::Button, &open).click();
    settle(&mut f.harness);
    let item = f
        .labels
        .t_with("aggregate.saved.open_item_n", &[("name", "with options")]);
    f.click_text(&item);
    click_key(&mut f, "aggregate.saved.open_confirm");
    wait_until(&mut f.harness, "the options restored", SHORT, |h| {
        let options = &h.state().state().aggregate.options;
        options.comment.as_deref() == Some("e2e comment") && options.allow_disk_use.is_none()
    });
}

fn dump_screen(f: &Fixture) -> String {
    support::screen::dump(&f.harness)
}
