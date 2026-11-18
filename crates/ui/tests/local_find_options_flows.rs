//! Click-driven flows of the find bar's Query options: the REAL app and runtime, driven through the
//! accessibility tree by `egui_kittest`, against a real MongoDB. Tier 1 (a local server): every
//! test writes to its own throwaway `cbit_*` database and drops it. Run with
//! `scripts/integration-tests.sh`, and ALWAYS point `CURLYBRACE_IT_MONGODB_URI` at a throwaway
//! server (the default is a developer's own MongoDB on 27017).
mod support;

use bson::{Document, doc};
use curlybrace_ui::viewmodel::BulkEntry;
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

/// `words` open, then More and the Query options section opened.
fn open_section(f: &mut Fixture) {
    browse(f, WORDS);
    click_key(f, "findbar.more");
    click_key(f, "query.options.toggle");
}

fn pick(f: &mut Fixture, combo_key: &str, item: &str) {
    let label = f.labels.t(combo_key);
    f.harness
        .get_by_role_and_label(Role::ComboBox, &label)
        .click();
    settle(&mut f.harness);
    f.click_text(item);
}

/// Collation `en`, strength 2, then a find of `{ "name": "abc" }`.
fn find_abc_under_a_collation(f: &mut Fixture) {
    click_key(f, "aggregate.options.collation.enabled");
    fill_key(f, "aggregate.options.collation.locale", "en");
    let second = f.labels.t("aggregate.options.collation.strength_value.2");
    pick(f, "aggregate.options.collation.strength", &second);
    let filter = f.labels.t("findbar.filter");
    fill(f, &filter, r#"{"name": "abc"}"#);
    click_key(f, "findbar.run");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_collation_set_in_the_section_changes_the_page() {
    let mut f = fixture!();
    open_section(&mut f);
    let filter = f.labels.t("findbar.filter");
    fill(&mut f, &filter, r#"{"name": "abc"}"#);
    click_key(&mut f, "findbar.run");
    wait_until(&mut f.harness, "the page of the binary match", SHORT, |h| {
        h.state()
            .state()
            .page
            .as_ref()
            .is_some_and(|p| p.docs.len() == 1)
    });
    find_abc_under_a_collation(&mut f);
    wait_until(
        &mut f.harness,
        "ABC beside abc under strength 2",
        SHORT,
        |h| {
            h.state()
                .state()
                .page
                .as_ref()
                .is_some_and(|p| p.docs.len() == 2)
        },
    );
    let page = f.harness.state().state().page.clone().expect("a page");
    let names: Vec<&str> = page
        .docs
        .iter()
        .filter_map(|d| d.get_str("name").ok())
        .collect();
    assert_eq!(names, ["abc", "ABC"]);
    wait_shows(&mut f, "Collation: en");
    assert_eq!(f.db.count(WORDS), 3, "a find never writes");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_hint_on_a_missing_index_says_so_in_the_bar() {
    let mut f = fixture!();
    open_section(&mut f);
    fill_key(&mut f, "aggregate.options.hint.label", "nope_1");
    click_key(&mut f, "findbar.run");
    let said = f.labels.t("errors.hint_not_found");
    wait_until(&mut f.harness, "the hint error in the bar", SHORT, |h| {
        h.query_all_by_label_contains(&said).next().is_some()
    });
    let state = f.harness.state().state();
    assert!(state.banner.is_none(), "no banner");
    assert!(state.page.is_some(), "the last page stays");
    assert!(shown(&f, "abc"));
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn the_hint_combo_lists_the_collection_s_indexes() {
    let mut f = fixture!();
    f.db.create_unique_index(WORDS, "name");
    open_section(&mut f);
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
    assert!(shown(&f, "name_1"), "{}", support::screen::dump(&f.harness));
    f.click_text("name_1");
    click_key(&mut f, "findbar.run");
    wait_until(&mut f.harness, "the page found with the hint", SHORT, |h| {
        h.query_all_by_label_contains("hint: name_1")
            .next()
            .is_some()
    });
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_lowered_time_limit_times_out_in_your_words() {
    let Some(mut f) = Fixture::local(test_name!()) else {
        return;
    };
    let docs: Vec<Document> = (0..200).map(|i| doc! {"_id": i, "name": "w"}).collect();
    f.db.insert(WORDS, docs);
    open_section(&mut f);
    fill_key(&mut f, "aggregate.options.max_time.label", "50");
    let filter = f.labels.t("findbar.filter");
    // 200 documents at 5 ms each of server-side JavaScript against a 50 ms limit.
    fill(
        &mut f,
        &filter,
        r#"{"$where": "function() { var t = Date.now(); while (Date.now() - t < 5) {} return true; }"}"#,
    );
    click_key(&mut f, "findbar.run");
    let said = f.labels.t("query.options.timeout_set");
    wait_until(&mut f.harness, "the time limit said as yours", SHORT, |h| {
        h.query_all_by_label_contains(&said).next().is_some()
    });
    assert!(f.harness.state().state().banner.is_none(), "no banner");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn paging_keeps_the_options() {
    let mut f = fixture!();
    open_section(&mut f);
    click_key(&mut f, "aggregate.options.collation.enabled");
    fill_key(&mut f, "aggregate.options.collation.locale", "en");
    let limit = f.labels.t("findbar.limit");
    fill(&mut f, &limit, "1");
    click_key(&mut f, "findbar.run");
    wait_until(&mut f.harness, "the first page", SHORT, |h| {
        h.query_all_by_label_contains("Collation: en")
            .next()
            .is_some()
    });
    click_key(&mut f, "findbar.next");
    wait_until(&mut f.harness, "the second page", SHORT, |h| {
        h.state().state().page_query.skip == 1
    });
    let options = &f.harness.state().state().page_query.options;
    assert_eq!(
        options.collation.as_ref().map(|c| c.locale.as_str()),
        Some("en")
    );
    assert!(shown(&f, "Collation: en"));
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn copy_as_code_carries_the_options() {
    let mut f = fixture!();
    open_section(&mut f);
    fill_key(&mut f, "aggregate.options.max_time.label", "5000");
    click_key(&mut f, "codegen.button");
    wait_until(&mut f.harness, "the code dialog", SHORT, |h| {
        h.state().state().code_export.dialog.is_some()
    });
    let text = f
        .harness
        .state()
        .state()
        .code_export
        .dialog
        .as_ref()
        .and_then(|d| d.text.clone().ok())
        .expect("the generated code");
    assert!(text.contains("maxTimeMS(5000)"), "{text}");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn update_matching_is_off_under_a_collation() {
    let mut f = fixture!();
    open_section(&mut f);
    find_abc_under_a_collation(&mut f);
    wait_until(
        &mut f.harness,
        "the page found with the collation",
        SHORT,
        |h| h.state().state().bulk_entry() == BulkEntry::Collated,
    );
    click_key(&mut f, "bulk.update_matching");
    settle(&mut f.harness);
    assert!(f.harness.state().state().bulk.is_none(), "no flow opens");
    assert_eq!(f.db.count(WORDS), 3);
}
