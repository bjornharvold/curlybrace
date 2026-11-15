//! The Plan tab's texts on stored explain documents, and the tree drawn from them.
use super::*;
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::explain::{AggHint, AggPlanOutcome, parse_aggregate_explain};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn plan(name: &str) -> AggPlan {
    let text = match name {
        "collscan" => {
            include_str!("../../../core/tests/fixtures/explain_aggregate/stages_collscan.json")
        }
        "ixscan" => {
            include_str!("../../../core/tests/fixtures/explain_aggregate/stages_ixscan.json")
        }
        "group_stats" => {
            include_str!("../../../core/tests/fixtures/explain_aggregate/stages_group_stats.json")
        }
        "sharded" => {
            include_str!("../../../core/tests/fixtures/explain_aggregate/sharded_stages.json")
        }
        other => panic!("unknown fixture {other}"),
    };
    let doc = editor_json_to_doc(text).expect("fixture parses");
    match parse_aggregate_explain(&doc) {
        AggPlanOutcome::Plan(plan) => plan,
        AggPlanOutcome::NotUnderstood => panic!("{name} is a plan"),
    }
}

fn english() -> I18n {
    I18n::english()
}

#[test]
fn only_the_reported_figures_are_named_in_a_fixed_order() {
    let i18n = english();
    let all = Figures {
        returned: Some(300),
        keys: Some(1200),
        docs: Some(1200),
        millis: Some(14),
    };
    assert_eq!(
        figures_text(all, &i18n),
        "300 returned, 1200 keys examined, 1200 documents examined, 14 ms"
    );
    let some = Figures {
        keys: Some(5),
        millis: Some(1),
        ..Figures::default()
    };
    assert_eq!(figures_text(some, &i18n), "5 keys examined, 1 ms");
    assert_eq!(figures_text(Figures::default(), &i18n), "");
}

#[test]
fn the_input_is_an_index_a_collection_scan_or_its_own_kind() {
    let i18n = english();
    let scan = plan("collscan").input.expect("input");
    assert_eq!(
        input_text(&scan, &i18n),
        i18n.t("aggregate.explain.input.collscan")
    );
    let indexed = plan("ixscan").input.expect("input");
    assert_eq!(
        input_text(&indexed, &i18n),
        i18n.t_with(
            "aggregate.explain.input.index",
            &[("index", indexed.index.as_deref().expect("index"))]
        )
    );
    let other = InputPlan {
        stage_kind: "TEXT_MATCH".into(),
        index: None,
        ..indexed
    };
    assert!(input_text(&other, &i18n).contains("TEXT_MATCH"));
}

#[test]
fn a_plan_without_counters_says_nothing_was_run_and_one_with_them_gives_the_totals() {
    let i18n = english();
    assert_eq!(
        summary_text(&plan("collscan"), &i18n),
        i18n.t("aggregate.explain.summary_none")
    );
    let measured = summary_text(&plan("group_stats"), &i18n);
    assert!(measured.contains("300 returned"), "{measured}");
    assert!(measured.contains("22 ms"), "{measured}");
}

#[test]
fn a_stage_shows_its_detail_in_brackets_when_it_has_one() {
    let i18n = english();
    let mut stage = plan("collscan").stages.remove(0);
    stage.detail = Some("customer".into());
    assert_eq!(
        stage_title(&stage, &i18n),
        format!("{} (customer)", stage.name)
    );
    stage.detail = None;
    assert_eq!(stage_title(&stage, &i18n), stage.name);
}

#[test]
fn every_hint_has_a_sentence_of_its_own() {
    let i18n = english();
    let hints = [
        AggHint::CollectionScan,
        AggHint::InMemorySort,
        AggHint::DiskSort,
        AggHint::ExaminedMuchMoreThanReturned,
        AggHint::LookupCollectionScan,
    ];
    let mut texts: Vec<String> = hints
        .iter()
        .map(|h| {
            assert_eq!(hint_key(*h), format!("aggregate.explain.hint.{}", h.key()));
            i18n.t(hint_key(*h))
        })
        .collect();
    assert!(
        texts.iter().all(|t| !t.starts_with('\u{27e6}')),
        "{texts:?}"
    );
    texts.sort();
    texts.dedup();
    assert_eq!(texts.len(), hints.len());
}

fn drawn(plan: AggPlan) -> Harness<'static, AggPlan> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(700.0, 500.0))
        .build_ui_state(
            |ui, plan: &mut AggPlan| show_plan(ui, plan, &english()),
            plan,
        );
    crate::theme::install(&harness.ctx);
    harness.run_steps(4);
    harness
}

#[test]
fn a_collection_scan_is_drawn_with_its_hint_in_words() {
    let i18n = english();
    let harness = drawn(plan("collscan"));
    harness.get_by_label(&i18n.t("aggregate.explain.input.collscan"));
    harness.get_by_label(&i18n.t("aggregate.explain.hints.heading"));
    harness.get_by_label(&i18n.t("aggregate.explain.hint.collection_scan"));
    harness.get_by_label_contains("$group");
}

#[test]
fn a_sharded_plan_is_one_group_per_shard() {
    let sharded = plan("sharded");
    assert!(sharded.shards.len() >= 2, "the fixture has two shards");
    let names: Vec<String> = sharded.shards.iter().map(|(n, _)| n.clone()).collect();
    let harness = drawn(sharded);
    let i18n = english();
    for name in names {
        let title = i18n.t_with("aggregate.explain.group.shard", &[("name", &name)]);
        harness.get_by_label(&title);
    }
}
