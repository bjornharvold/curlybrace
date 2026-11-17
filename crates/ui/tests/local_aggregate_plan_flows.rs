//! Click-driven flows of the Plan tab's follow-ups: the REAL app and runtime, driven through the
//! accessibility tree by `egui_kittest`, against a real MongoDB. Tier 1 (a local server): every
//! test writes to its own throwaway `cbit_*` database and drops it. Run with
//! `scripts/integration-tests.sh`, and ALWAYS point `CURLYBRACE_IT_MONGODB_URI` at a throwaway
//! server (the default is a developer's own MongoDB on 27017).
mod support;

use bson::doc;
use curlybrace_core::pipeline_options::HintOption;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use support::actions::*;
use support::fixture::Fixture;
use support::wait::{SHORT, settle, wait_until};

const ORDERS: &str = "orders";

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
            ORDERS,
            vec![
                doc! {"_id": 1_i32, "status": "A", "total": 10_i32, "name": "x"},
                doc! {"_id": 2_i32, "status": "B", "total": 99_i32, "name": "y"},
                doc! {"_id": 3_i32, "status": "A", "total": 30_i32, "name": "z"},
            ],
        );
        f
    }};
}

fn shown(f: &Fixture, text: &str) -> bool {
    f.harness.query_all_by_label_contains(text).next().is_some()
}

/// The Aggregate tab of `orders` with `$match` on the status and a `$sort` on the name, which no index serves, explained
/// (plan only): the Plan tab shows the plan once it arrived.
fn explained(f: &mut Fixture) {
    browse(f, ORDERS);
    click_key(f, "aggregate.tab");
    for (n, text) in [
        ("1", r#"{ "$match": { "status": "A" } }"#),
        ("2", r#"{ "$sort": { "name": 1 } }"#),
    ] {
        let add = f.labels.t("aggregate.add_stage");
        f.harness
            .get_by_role_and_label(Role::ComboBox, &add)
            .click();
        settle(&mut f.harness);
        f.click_text(if n == "1" { "$match" } else { "$sort" });
        let editor = f.labels.t_with("aggregate.stage.editor_n", &[("n", n)]);
        fill(f, &editor, text);
    }
    click_key(f, "aggregate.explain.button");
    wait_until(&mut f.harness, "the plan", SHORT, |h| {
        h.state().state().aggregate.explain.done().is_some()
    });
}

/// Waits for the Try an index combo (the index list was read) and picks `name`.
fn try_index(f: &mut Fixture, name: &str) {
    let label = f.labels.t("aggregate.options.explain.try_index");
    wait_until(&mut f.harness, "the Try an index picker", SHORT, |h| {
        h.query_all_by_role_and_label(Role::ComboBox, &label)
            .next()
            .is_some()
    });
    f.harness
        .get_by_role_and_label(Role::ComboBox, &label)
        .click();
    settle(&mut f.harness);
    f.click_text(name);
}

fn hint(f: &Fixture) -> Option<HintOption> {
    f.harness.state().state().aggregate.options.hint.clone()
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn try_an_index_re_explains_through_that_index() {
    let mut f = fixture!();
    f.db.create_index(ORDERS, doc! { "status": 1_i32 });
    f.db.create_index(ORDERS, doc! { "total": -1_i32 });
    explained(&mut f);
    let first = f.harness.state().state().aggregate.explain.done().cloned();
    assert!(first.is_some_and(|d| d.options.hint.is_none()));
    try_index(&mut f, "total_-1");
    wait_until(&mut f.harness, "the plan made with the hint", SHORT, |h| {
        h.state()
            .state()
            .aggregate
            .explain
            .done()
            .is_some_and(|d| d.options.hint == Some(HintOption::Name("total_-1".into())))
    });
    assert_eq!(hint(&f), Some(HintOption::Name("total_-1".into())));
    let reads = f
        .labels
        .t_with("aggregate.explain.input.index", &[("index", "total_-1")]);
    wait_until(&mut f.harness, "the input naming the index", SHORT, |h| {
        h.query_all_by_label_contains(&reads).next().is_some()
    });
    assert_eq!(f.db.count(ORDERS), 3, "an explain never writes");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn clear_hint_explains_without_it() {
    let mut f = fixture!();
    f.db.create_index(ORDERS, doc! { "status": 1_i32 });
    f.db.create_index(ORDERS, doc! { "total": -1_i32 });
    explained(&mut f);
    try_index(&mut f, "total_-1");
    wait_until(&mut f.harness, "the hinted plan", SHORT, |h| {
        h.state()
            .state()
            .aggregate
            .explain
            .done()
            .is_some_and(|d| d.options.hint.is_some())
    });
    click_key(&mut f, "aggregate.options.explain.clear_hint");
    wait_until(&mut f.harness, "the plan made without it", SHORT, |h| {
        h.state()
            .state()
            .aggregate
            .explain
            .done()
            .is_some_and(|d| d.options.hint.is_none())
    });
    assert_eq!(hint(&f), None);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn create_index_from_the_suggestion_builds_it_and_lists_it() {
    let mut f = fixture!();
    explained(&mut f);
    assert!(f.db.index_names(ORDERS).iter().all(|n| n == "_id_"));
    click_key(&mut f, "aggregate.options.explain.create_index");
    let create = f.labels.t("indexes.builder.create");
    wait_until(&mut f.harness, "the index builder", SHORT, |h| {
        h.query_all_by_role_and_label(Role::Button, &create)
            .next()
            .is_some()
    });
    assert!(
        f.harness.state().state().indexes.builder.is_some(),
        "pre-filled, nothing built yet"
    );
    assert!(f.db.index_names(ORDERS).iter().all(|n| n == "_id_"));
    f.harness
        .get_by_role_and_label(Role::Button, &create)
        .click();
    wait_until(&mut f.harness, "the index on the server", SHORT, |_| {
        f.db.index_names(ORDERS)
            .contains(&"status_1_name_1".to_string())
    });
    // The hint picker asks again, so the new index is in it.
    let label = f.labels.t("aggregate.options.explain.try_index");
    wait_until(
        &mut f.harness,
        "the picker with the new index",
        SHORT,
        |h| {
            h.state()
                .state()
                .aggregate
                .options_ui
                .hint
                .names()
                .iter()
                .any(|n| n == "status_1_name_1")
        },
    );
    f.harness
        .get_by_role_and_label(Role::ComboBox, &label)
        .click();
    settle(&mut f.harness);
    assert!(shown(&f, "status_1_name_1"));
}
